#![allow(unsafe_code)]

use super::*;
use oxyos_ipc::{Listener, SharedBuffer};
use oxyos_protocol::{
    AccessibleAction, DismissReason, GesturePhase, PointerKind, PreeditText, ScrollSource,
    ServerConnection, MAX_PREFERENCE_BYTES, NO_ACCESSIBLE_PARENT,
};
use std::mem::{offset_of, size_of};

const HEADER: &str = include_str!("../include/oxyos_client.h");

/// `#define NAME value` from the header.
fn define(name: &str) -> i64 {
    let line = HEADER
        .lines()
        .find(|line| line.split_whitespace().nth(1) == Some(name))
        .unwrap_or_else(|| panic!("{name} is not defined in oxyos_client.h"));
    let value = line
        .split_whitespace()
        .nth(2)
        .unwrap()
        .trim_matches(|c| c == '(' || c == ')');
    match value.strip_prefix("0x") {
        Some(hex) => i64::from_str_radix(hex, 16).unwrap(),
        None => value.parse().unwrap(),
    }
}

/// A C struct field: its name, element size and element count (1 unless an array).
struct Field {
    name: String,
    size: usize,
    count: usize,
}

/// The fields of `typedef struct NAME { ... }`, in order.
fn fields(name: &str) -> Vec<Field> {
    let start = HEADER.find(&format!("typedef struct {name} {{")).unwrap();
    let body = &HEADER[start..];
    let mut body = &body[body.find('{').unwrap() + 1..body.find('}').unwrap()];
    // Without its comments.
    let mut code = String::new();
    while let Some(at) = body.find("/*") {
        code.push_str(&body[..at]);
        body = &body[at + body[at..].find("*/").unwrap() + 2..];
    }
    code.push_str(body);
    code.split(';')
        .filter_map(|field| {
            // The declarator is the last word; any word before it ending in '*' makes a pointer
            // (`const char* const* names`).
            let words: Vec<&str> = field
                .split_whitespace()
                .filter(|word| *word != "const")
                .collect();
            let (&declarator, kinds) = words.split_last()?;
            let kind = if kinds.iter().any(|word| word.ends_with('*')) {
                "*"
            } else {
                *kinds.first()?
            };
            let size = match kind {
                "uint8_t" | "char" | "bool" => 1,
                "uint16_t" => 2,
                "int" | "int32_t" | "uint32_t" | "float" => 4,
                "uint64_t" | "int64_t" | "size_t" | "double" => 8,
                pointer if pointer.ends_with('*') => 8,
                other => panic!("unexpected field type {other}"),
            };
            let (name, count) = match declarator.split_once('[') {
                Some((name, count)) => (name, count.trim_end_matches(']').parse().unwrap()),
                None => (declarator, 1),
            };
            Some(Field {
                name: name.to_string(),
                size,
                count,
            })
        })
        .collect()
}

fn names(fields: &[Field]) -> Vec<&str> {
    fields.iter().map(|field| field.name.as_str()).collect()
}

/// C offsets of fields laid out in order with natural alignment, and the struct's size.
fn c_offsets(fields: &[Field]) -> (Vec<usize>, usize) {
    let (mut offsets, mut at, mut align) = (Vec::new(), 0_usize, 1);
    for field in fields {
        at = at.next_multiple_of(field.size);
        offsets.push(at);
        at += field.size * field.count;
        align = align.max(field.size);
    }
    (offsets, at.next_multiple_of(align))
}

#[test]
fn the_header_matches_the_abi() {
    for (name, value) in [
        ("OXYOS_EVENT_CONFIGURE", OXYOS_EVENT_CONFIGURE as i64),
        ("OXYOS_EVENT_POPUP_DONE", OXYOS_EVENT_POPUP_DONE.into()),
        ("OXYOS_EVENT_POINTER", OXYOS_EVENT_POINTER.into()),
        ("OXYOS_EVENT_KEY", OXYOS_EVENT_KEY.into()),
        ("OXYOS_EVENT_FOCUS", OXYOS_EVENT_FOCUS.into()),
        ("OXYOS_EVENT_TOUCH", OXYOS_EVENT_TOUCH.into()),
        ("OXYOS_EVENT_FEEDBACK", OXYOS_EVENT_FEEDBACK.into()),
        ("OXYOS_EVENT_RELEASED", OXYOS_EVENT_RELEASED.into()),
        ("OXYOS_EVENT_SCROLL", OXYOS_EVENT_SCROLL.into()),
        ("OXYOS_EVENT_HOLD", OXYOS_EVENT_HOLD.into()),
        ("OXYOS_EVENT_TEXT", OXYOS_EVENT_TEXT.into()),
        ("OXYOS_EVENT_PREEDIT", OXYOS_EVENT_PREEDIT.into()),
        (
            "OXYOS_EVENT_ACCESSIBILITY_ACTION",
            OXYOS_EVENT_ACCESSIBILITY_ACTION.into(),
        ),
        (
            "OXYOS_EVENT_CLIPBOARD_TYPES",
            OXYOS_EVENT_CLIPBOARD_TYPES.into(),
        ),
        (
            "OXYOS_EVENT_CLIPBOARD_TYPE",
            OXYOS_EVENT_CLIPBOARD_TYPE.into(),
        ),
        (
            "OXYOS_EVENT_CLIPBOARD_EMPTY",
            OXYOS_EVENT_CLIPBOARD_EMPTY.into(),
        ),
        ("OXYOS_ACTION_ACTIVATE", AccessibleAction::Activate as i64),
        ("OXYOS_ACTION_FOCUS", AccessibleAction::Focus as i64),
        ("OXYOS_ACTION_INCREMENT", AccessibleAction::Increment as i64),
        ("OXYOS_ACTION_DECREMENT", AccessibleAction::Decrement as i64),
        (
            "OXYOS_ACTION_SCROLL_FORWARD",
            AccessibleAction::ScrollForward as i64,
        ),
        (
            "OXYOS_ACTION_SCROLL_BACKWARD",
            AccessibleAction::ScrollBackward as i64,
        ),
        ("OXYOS_ACTION_DISMISS", AccessibleAction::Dismiss as i64),
        ("OXYOS_ACTION_CUSTOM", AccessibleAction::Custom as i64),
        (
            "OXYOS_ACTION_SET_SELECTION",
            AccessibleAction::SetSelection as i64,
        ),
        (
            "OXYOS_OFFERS_INCREMENT",
            oxyos_protocol::AccessibleActions::INCREMENT.into(),
        ),
        (
            "OXYOS_OFFERS_DECREMENT",
            oxyos_protocol::AccessibleActions::DECREMENT.into(),
        ),
        (
            "OXYOS_OFFERS_SCROLL_FORWARD",
            oxyos_protocol::AccessibleActions::SCROLL_FORWARD.into(),
        ),
        (
            "OXYOS_OFFERS_SCROLL_BACKWARD",
            oxyos_protocol::AccessibleActions::SCROLL_BACKWARD.into(),
        ),
        (
            "OXYOS_OFFERS_DISMISS",
            oxyos_protocol::AccessibleActions::DISMISS.into(),
        ),
        (
            "OXYOS_OFFERS_SET_SELECTION",
            oxyos_protocol::AccessibleActions::SET_SELECTION.into(),
        ),
        ("OXYOS_POLITE", oxyos_protocol::Politeness::Polite as i64),
        (
            "OXYOS_ASSERTIVE",
            oxyos_protocol::Politeness::Assertive as i64,
        ),
        (
            "OXYOS_STATE_LIVE_POLITE",
            AccessibleState::LIVE_POLITE.into(),
        ),
        (
            "OXYOS_STATE_LIVE_ASSERTIVE",
            AccessibleState::LIVE_ASSERTIVE.into(),
        ),
        ("OXYOS_NO_ACCESSIBLE_PARENT", NO_ACCESSIBLE_PARENT.into()),
        ("OXYOS_STATE_DISABLED", AccessibleState::DISABLED.into()),
        ("OXYOS_STATE_SELECTED", AccessibleState::SELECTED.into()),
        ("OXYOS_STATE_CHECKED", AccessibleState::CHECKED.into()),
        ("OXYOS_STATE_MIXED", AccessibleState::MIXED.into()),
        ("OXYOS_STATE_EXPANDED", AccessibleState::EXPANDED.into()),
        ("OXYOS_STATE_COLLAPSED", AccessibleState::COLLAPSED.into()),
        ("OXYOS_STATE_BUSY", AccessibleState::BUSY.into()),
        ("OXYOS_STATE_FOCUSED", AccessibleState::FOCUSED.into()),
        ("OXYOS_POINTER_ENTER", PointerKind::Enter as i64),
        ("OXYOS_POINTER_LEAVE", PointerKind::Leave as i64),
        ("OXYOS_POINTER_MOTION", PointerKind::Motion as i64),
        ("OXYOS_POINTER_PRESS", PointerKind::Press as i64),
        ("OXYOS_POINTER_RELEASE", PointerKind::Release as i64),
        ("OXYOS_POINTER_CANCEL", PointerKind::Cancel as i64),
        (
            "OXYOS_OUTCOME_PRESENTED",
            oxyos_protocol::Outcome::Presented as i64,
        ),
        (
            "OXYOS_OUTCOME_DISCARDED",
            oxyos_protocol::Outcome::Discarded as i64,
        ),
        ("OXYOS_FORMAT_XRGB8888", OXYOS_FORMAT_XRGB8888.into()),
        ("OXYOS_FORMAT_ARGB8888", OXYOS_FORMAT_ARGB8888.into()),
        ("OXYOS_POLL_EVENT", OXYOS_POLL_EVENT.into()),
        ("OXYOS_POLL_EMPTY", OXYOS_POLL_EMPTY.into()),
        ("OXYOS_POLL_CLOSED", OXYOS_POLL_CLOSED.into()),
        ("OXYOS_ERROR", OXYOS_ERROR.into()),
    ] {
        assert_eq!(define(name), value, "{name}");
    }

    // Every role, in both directions: the header's names and the protocol's values agree.
    let roles: Vec<(&str, i64)> = HEADER
        .lines()
        .filter_map(|line| line.strip_prefix("#define OXYOS_ROLE_"))
        .map(|rest| {
            let (name, value) = rest.split_once(' ').unwrap();
            (name, value.parse().unwrap())
        })
        .collect();
    for (name, value) in &roles {
        let role = AccessibleRole::from_raw(*value as u8).unwrap_or_else(|| panic!("{name}"));
        let rust: String = format!("{role:?}")
            .chars()
            .flat_map(|c| {
                let upper = c.is_ascii_uppercase();
                [(upper).then_some('_'), Some(c.to_ascii_uppercase())]
            })
            .flatten()
            .collect();
        assert_eq!(rust.trim_start_matches('_'), *name);
    }
    assert!(AccessibleRole::from_raw(roles.len() as u8).is_none());

    let node = fields("OxyOSAccessibleNode");
    assert_eq!(
        names(&node),
        [
            "parent",
            "role",
            "state",
            "x",
            "y",
            "width",
            "height",
            "name",
            "value",
            "actions",
            "custom_actions",
            "has_selection",
            "selection_start",
            "selection_end"
        ]
    );
    let rust = [
        offset_of!(OxyOSAccessibleNode, parent),
        offset_of!(OxyOSAccessibleNode, role),
        offset_of!(OxyOSAccessibleNode, state),
        offset_of!(OxyOSAccessibleNode, x),
        offset_of!(OxyOSAccessibleNode, y),
        offset_of!(OxyOSAccessibleNode, width),
        offset_of!(OxyOSAccessibleNode, height),
        offset_of!(OxyOSAccessibleNode, name),
        offset_of!(OxyOSAccessibleNode, value),
        offset_of!(OxyOSAccessibleNode, actions),
        offset_of!(OxyOSAccessibleNode, custom_actions),
        offset_of!(OxyOSAccessibleNode, has_selection),
        offset_of!(OxyOSAccessibleNode, selection_start),
        offset_of!(OxyOSAccessibleNode, selection_end),
    ];
    assert_eq!(
        c_offsets(&node),
        (rust.to_vec(), size_of::<OxyOSAccessibleNode>())
    );

    let rect = fields("OxyOSRect");
    assert_eq!(names(&rect), ["x", "y", "width", "height"]);
    let rust = [
        offset_of!(OxyOSRect, x),
        offset_of!(OxyOSRect, y),
        offset_of!(OxyOSRect, width),
        offset_of!(OxyOSRect, height),
    ];
    assert_eq!(c_offsets(&rect), (rust.to_vec(), size_of::<OxyOSRect>()));

    let entry = fields("OxyOSClipboardEntry");
    assert_eq!(names(&entry), ["mime", "mime_length", "data", "length"]);
    let rust = [
        offset_of!(OxyOSClipboardEntry, mime),
        offset_of!(OxyOSClipboardEntry, mime_length),
        offset_of!(OxyOSClipboardEntry, data),
        offset_of!(OxyOSClipboardEntry, length),
    ];
    assert_eq!(
        c_offsets(&entry),
        (rust.to_vec(), size_of::<OxyOSClipboardEntry>())
    );

    let event = fields("OxyOSEvent");
    let rust = [
        offset_of!(OxyOSEvent, kind),
        offset_of!(OxyOSEvent, surface),
        offset_of!(OxyOSEvent, serial),
        offset_of!(OxyOSEvent, x),
        offset_of!(OxyOSEvent, y),
        offset_of!(OxyOSEvent, width),
        offset_of!(OxyOSEvent, height),
        offset_of!(OxyOSEvent, scale_120),
        offset_of!(OxyOSEvent, detail),
        offset_of!(OxyOSEvent, code),
        offset_of!(OxyOSEvent, scale_65536),
        offset_of!(OxyOSEvent, rotation_256),
        offset_of!(OxyOSEvent, flag),
        offset_of!(OxyOSEvent, text),
        offset_of!(OxyOSEvent, time_ns),
        offset_of!(OxyOSEvent, modifiers),
    ];
    assert_eq!(
        event.iter().find(|f| f.name == "text").map(|f| f.count),
        Some(TEXT_CHUNK_BYTES + 1)
    );
    assert_eq!(
        names(&event),
        [
            "kind",
            "surface",
            "serial",
            "x",
            "y",
            "width",
            "height",
            "scale_120",
            "detail",
            "code",
            "scale_65536",
            "rotation_256",
            "flag",
            "text",
            "time_ns",
            "modifiers"
        ]
    );
    assert_eq!(c_offsets(&event), (rust.to_vec(), size_of::<OxyOSEvent>()));

    let positioner = fields("OxyOSPositioner");
    assert_eq!(
        names(&positioner),
        [
            "anchor_x",
            "anchor_y",
            "anchor_width",
            "anchor_height",
            "width",
            "height",
            "anchor",
            "gravity",
            "adjust",
            "offset_x",
            "offset_y"
        ]
    );
    let rust = [
        offset_of!(OxyOSPositioner, anchor_x),
        offset_of!(OxyOSPositioner, anchor_y),
        offset_of!(OxyOSPositioner, anchor_width),
        offset_of!(OxyOSPositioner, anchor_height),
        offset_of!(OxyOSPositioner, width),
        offset_of!(OxyOSPositioner, height),
        offset_of!(OxyOSPositioner, anchor),
        offset_of!(OxyOSPositioner, gravity),
        offset_of!(OxyOSPositioner, adjust),
        offset_of!(OxyOSPositioner, offset_x),
        offset_of!(OxyOSPositioner, offset_y),
    ];
    assert_eq!(
        c_offsets(&positioner),
        (rust.to_vec(), size_of::<OxyOSPositioner>())
    );
}

#[test]
fn events_convert_field_by_field() {
    let pointer = convert(&Event::Pointer {
        surface: 1,
        serial: 9,
        kind: PointerKind::Press,
        x: 256 * 10,
        y: 256 * 20,
        button: 272,
        time_ns: 5_000_000_123,
        modifiers: OXYOS_MODIFIER_CONTROL | OXYOS_MODIFIER_ALT,
    })
    .unwrap();
    assert_eq!(
        pointer,
        OxyOSEvent {
            kind: OXYOS_EVENT_POINTER,
            surface: 1,
            serial: 9,
            x: 2560,
            y: 5120,
            detail: PointerKind::Press as u32,
            code: 272,
            time_ns: 5_000_000_123,
            modifiers: OXYOS_MODIFIER_CONTROL | OXYOS_MODIFIER_ALT,
            ..OxyOSEvent::default()
        }
    );
    let done = convert(&Event::PopupDone {
        surface: 2,
        reason: DismissReason::OutsidePress,
    })
    .unwrap();
    assert_eq!(done.detail, DismissReason::OutsidePress as u32);
    let text = convert(&Event::Text {
        surface: 1,
        serial: 3,
        text: oxyos_protocol::TextChunk::new("héllo").unwrap(),
    })
    .unwrap();
    assert_eq!(text.kind, OXYOS_EVENT_TEXT);
    assert_eq!(
        CStr::from_bytes_until_nul(&text.text).unwrap().to_str(),
        Ok("héllo")
    );
    let scroll = convert(&Event::Scroll {
        surface: 1,
        serial: 4,
        dx: 0,
        dy: 512,
        time_ns: 5_000_000_123,
        source: ScrollSource::Wheel,
        stop: false,
    })
    .unwrap();
    assert_eq!(
        (scroll.kind, scroll.x, scroll.y, scroll.detail, scroll.flag),
        (OXYOS_EVENT_SCROLL, 0, 512, OXYOS_SCROLL_WHEEL, 0)
    );
    let stop = convert(&Event::Scroll {
        surface: 1,
        serial: 5,
        dx: 0,
        dy: 0,
        time_ns: 6_000_000_000,
        source: ScrollSource::Finger,
        stop: true,
    })
    .unwrap();
    assert_eq!(
        (stop.kind, stop.x, stop.y, stop.detail, stop.flag),
        (OXYOS_EVENT_SCROLL, 0, 0, OXYOS_SCROLL_FINGER, 1)
    );
    assert_eq!(convert(&Event::Welcome { version: 2 }), None);
}

#[test]
fn positioners_with_unknown_values_are_rejected() {
    let valid = OxyOSPositioner {
        anchor_width: 1,
        anchor_height: 1,
        width: 10,
        height: 10,
        anchor: 8,
        gravity: 8,
        adjust: 15,
        ..OxyOSPositioner::default()
    };
    assert!(positioner(&valid).is_some());
    assert!(positioner(&OxyOSPositioner { anchor: 9, ..valid }).is_none());
    assert!(positioner(&OxyOSPositioner {
        gravity: 200,
        ..valid
    })
    .is_none());
    assert!(positioner(&OxyOSPositioner {
        adjust: 128,
        ..valid
    })
    .is_none());
}

#[test]
fn a_session_through_the_c_abi() {
    let directory = std::env::temp_dir().join(format!("oxyos-client-ffi-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("s");
    let listener = Listener::bind(&socket).unwrap();
    let compositor = std::thread::spawn(move || {
        let server = listener.accept().unwrap();
        let mut protocol = ServerConnection::new();
        let (hello, _) = server.recv_request().unwrap();
        server
            .send_event(&protocol.receive(&hello).unwrap().unwrap(), None)
            .unwrap();
        assert_eq!(
            server.recv_request().unwrap().0,
            Request::CreateToplevel {
                surface: 1,
                size: Size::new(2, 1)
            }
        );
        server
            .send_event(
                &Event::Configure {
                    surface: 1,
                    serial: 5,
                    rect: Rect::new(0, 0, 2, 1),
                    scale_120: 120,
                    fullscreen: false,
                    maximized: false,
                },
                None,
            )
            .unwrap();
        let (attach, fd) = server.recv_request().unwrap();
        let shared = SharedBuffer::import(fd.unwrap(), attach.buffer_layout().unwrap()).unwrap();
        let (submit, _) = server.recv_request().unwrap();
        let mut row = vec![0; 8];
        shared.read_row(0, &mut row).unwrap();
        (submit, row)
    });

    let path = CString::new(socket.to_str().unwrap()).unwrap();
    // SAFETY: the client pointer comes from oxyos_client_connect, is used by this thread only and
    // is closed once at the end; every other pointer is to a live local of the right type.
    unsafe {
        let client = oxyos_client_connect(path.as_ptr());
        assert!(!client.is_null());
        assert!(oxyos_client_fd(client) >= 0);
        assert_eq!(oxyos_client_create_toplevel(client, 1, 2, 1), 0);
        let mut event = OxyOSEvent::default();
        let configure = loop {
            match oxyos_client_poll(client, &mut event) {
                OXYOS_POLL_EVENT => break event,
                OXYOS_POLL_EMPTY => std::thread::yield_now(),
                other => panic!("poll returned {other}"),
            }
        };
        assert_eq!(
            (configure.kind, configure.serial),
            (OXYOS_EVENT_CONFIGURE, 5)
        );
        assert_eq!(
            (configure.width, configure.height, configure.scale_120),
            (2, 1, 120)
        );
        let pixels = [1u8, 2, 3, 0, 4, 5, 6, 0];
        let present = |width, height| {
            oxyos_client_present(
                client,
                1,
                5,
                width,
                height,
                OXYOS_FORMAT_XRGB8888,
                pixels.as_ptr(),
                pixels.len(),
            )
        };
        assert_eq!(present(2, 1), 1);
        // A frame whose pixels do not match its size is refused, with a reason.
        assert_eq!(present(4, 4), -1);
        let reason = CStr::from_ptr(oxyos_last_error()).to_str().unwrap();
        assert!(reason.contains("extent"), "{reason}");

        let (submit, row) = compositor.join().unwrap();
        assert_eq!(
            submit,
            Request::Submit {
                surface: 1,
                sequence: 1,
                buffer: 1,
                configure: 5,
                extent: Size::new(2, 1)
            }
        );
        assert_eq!(row, pixels);
        assert_eq!(oxyos_client_poll(client, &mut event), OXYOS_POLL_CLOSED);
        oxyos_client_close(client);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn accessibility_through_the_c_abi() {
    let directory =
        std::env::temp_dir().join(format!("oxyos-client-ffi-a11y-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("s");
    let listener = Listener::bind(&socket).unwrap();
    let compositor = std::thread::spawn(move || {
        let server = listener.accept().unwrap();
        let mut protocol = ServerConnection::new();
        let (hello, _) = server.recv_request().unwrap();
        server
            .send_event(&protocol.receive(&hello).unwrap().unwrap(), None)
            .unwrap();
        let mut requests = Vec::new();
        loop {
            let (request, _) = server.recv_request().unwrap();
            let done = matches!(request, Request::AccessibilityCommit { .. });
            requests.push(request);
            if done {
                break;
            }
        }
        server
            .send_event(
                &Event::AccessibilityAction {
                    surface: 1,
                    serial: 77,
                    index: 1,
                    action: AccessibleAction::SetSelection,
                    custom: 0,
                    start: 2,
                    end: 5,
                },
                None,
            )
            .unwrap();
        requests
    });

    let path = CString::new(socket.to_str().unwrap()).unwrap();
    let menu = CString::new("Message actions").unwrap();
    let reply = CString::new("Reply").unwrap();
    let node = |parent, role, state, name: &CString| OxyOSAccessibleNode {
        parent,
        role,
        state,
        x: 8,
        y: 8,
        width: 184,
        height: 40,
        name: name.as_ptr(),
        value: std::ptr::null(),
        actions: 0,
        custom_actions: std::ptr::null(),
        has_selection: 0,
        selection_start: 0,
        selection_end: 0,
    };
    let nodes = [
        node(NO_ACCESSIBLE_PARENT, AccessibleRole::Menu as u8, 0, &menu),
        node(
            0,
            AccessibleRole::MenuItem as u8,
            AccessibleState::FOCUSED,
            &reply,
        ),
    ];
    // SAFETY: the client pointer comes from oxyos_client_connect and is closed once; `nodes`
    // and the strings they point to outlive the calls.
    unsafe {
        let client = oxyos_client_connect(path.as_ptr());
        assert!(!client.is_null());
        // An unknown role is refused before anything is sent.
        let bad = [node(NO_ACCESSIBLE_PARENT, 200, 0, &menu)];
        assert_eq!(
            oxyos_client_publish_accessibility(client, 1, bad.as_ptr(), 1),
            OXYOS_ERROR
        );
        assert_eq!(
            oxyos_client_publish_accessibility(client, 1, nodes.as_ptr(), 2),
            0
        );
        let mut event = OxyOSEvent::default();
        let action = loop {
            match oxyos_client_poll(client, &mut event) {
                OXYOS_POLL_EVENT => break event,
                OXYOS_POLL_EMPTY => std::thread::yield_now(),
                other => panic!("poll returned {other}"),
            }
        };
        assert_eq!(
            (
                action.kind,
                action.surface,
                action.serial,
                action.code,
                action.detail,
                action.x,
                action.y
            ),
            (
                OXYOS_EVENT_ACCESSIBILITY_ACTION,
                1,
                77,
                1,
                AccessibleAction::SetSelection as u32,
                2,
                5
            )
        );
        let requests = compositor.join().unwrap();
        assert_eq!(
            requests[0],
            Request::AccessibilityBegin {
                surface: 1,
                nodes: 2
            }
        );
        assert!(requests.iter().any(|r| matches!(
            r,
            Request::AccessibleText { index: 1, text, .. } if text.as_str() == "Reply"
        )));
        oxyos_client_close(client);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn null_arguments_fail_instead_of_crashing() {
    // SAFETY: NULL is accepted by every entry point and reported as an error.
    unsafe {
        assert!(oxyos_client_connect(std::ptr::null()).is_null());
        assert_eq!(oxyos_client_fd(std::ptr::null_mut()), OXYOS_ERROR);
        assert_eq!(
            oxyos_client_create_toplevel(std::ptr::null_mut(), 1, 1, 1),
            OXYOS_ERROR
        );
        assert_eq!(
            oxyos_client_poll(std::ptr::null_mut(), std::ptr::null_mut()),
            OXYOS_ERROR
        );
        assert_eq!(
            oxyos_client_reposition(std::ptr::null_mut(), 2, std::ptr::null()),
            OXYOS_ERROR
        );
        assert_eq!(
            oxyos_client_publish_accessibility(std::ptr::null_mut(), 1, std::ptr::null(), 0),
            OXYOS_ERROR
        );
        assert_eq!(
            oxyos_client_publish_accessibility(std::ptr::null_mut(), 1, std::ptr::null(), 3),
            OXYOS_ERROR
        );
        oxyos_client_close(std::ptr::null_mut());
    }
}

#[test]
fn the_header_matches_the_network_abi() {
    use net::*;
    for (name, value) in [
        ("OXYOS_NET_RESPONSE", i64::from(OXYOS_NET_RESPONSE)),
        ("OXYOS_NET_CHUNK", OXYOS_NET_CHUNK.into()),
        ("OXYOS_NET_DATA", OXYOS_NET_DATA.into()),
        ("OXYOS_NET_COMPLETE", OXYOS_NET_COMPLETE.into()),
        ("OXYOS_NET_SENT", OXYOS_NET_SENT.into()),
        ("OXYOS_NET_ANSWER", OXYOS_NET_ANSWER.into()),
        ("OXYOS_NET_EMPTY", OXYOS_NET_EMPTY.into()),
    ] {
        assert_eq!(define(name), value, "{name}");
    }

    let header = fields("OxyOSNetHeader");
    assert_eq!(
        names(&header),
        ["name", "name_length", "value", "value_length"]
    );
    let rust = [
        offset_of!(OxyOSNetHeader, name),
        offset_of!(OxyOSNetHeader, name_length),
        offset_of!(OxyOSNetHeader, value),
        offset_of!(OxyOSNetHeader, value_length),
    ];
    assert_eq!(
        c_offsets(&header),
        (rust.to_vec(), size_of::<OxyOSNetHeader>())
    );

    let part = fields("OxyOSNetPart");
    assert_eq!(names(&part), ["bytes", "length", "fd", "offset"]);
    let rust = [
        offset_of!(OxyOSNetPart, bytes),
        offset_of!(OxyOSNetPart, length),
        offset_of!(OxyOSNetPart, fd),
        offset_of!(OxyOSNetPart, offset),
    ];
    assert_eq!(c_offsets(&part), (rust.to_vec(), size_of::<OxyOSNetPart>()));

    let request = fields("OxyOSNetRequest");
    assert_eq!(
        names(&request),
        [
            "id",
            "method",
            "method_length",
            "url",
            "url_length",
            "headers",
            "header_count",
            "body",
            "body_length",
            "timeout_ms",
            "has_body",
            "with_credentials",
            "incremental",
            "upload_progress",
            "parts",
            "part_count"
        ]
    );
    let rust = [
        offset_of!(OxyOSNetRequest, id),
        offset_of!(OxyOSNetRequest, method),
        offset_of!(OxyOSNetRequest, method_length),
        offset_of!(OxyOSNetRequest, url),
        offset_of!(OxyOSNetRequest, url_length),
        offset_of!(OxyOSNetRequest, headers),
        offset_of!(OxyOSNetRequest, header_count),
        offset_of!(OxyOSNetRequest, body),
        offset_of!(OxyOSNetRequest, body_length),
        offset_of!(OxyOSNetRequest, timeout_ms),
        offset_of!(OxyOSNetRequest, has_body),
        offset_of!(OxyOSNetRequest, with_credentials),
        offset_of!(OxyOSNetRequest, incremental),
        offset_of!(OxyOSNetRequest, upload_progress),
        offset_of!(OxyOSNetRequest, parts),
        offset_of!(OxyOSNetRequest, part_count),
    ];
    assert_eq!(
        c_offsets(&request),
        (rust.to_vec(), size_of::<OxyOSNetRequest>())
    );

    let answer = fields("OxyOSNetAnswer");
    assert_eq!(
        names(&answer),
        [
            "kind",
            "status",
            "id",
            "headers",
            "header_count",
            "url",
            "url_length",
            "bytes",
            "length",
            "progress",
            "total",
            "content_type",
            "content_type_length",
            "error",
            "error_length",
            "timed_out"
        ]
    );
    let rust = [
        offset_of!(OxyOSNetAnswer, kind),
        offset_of!(OxyOSNetAnswer, status),
        offset_of!(OxyOSNetAnswer, id),
        offset_of!(OxyOSNetAnswer, headers),
        offset_of!(OxyOSNetAnswer, header_count),
        offset_of!(OxyOSNetAnswer, url),
        offset_of!(OxyOSNetAnswer, url_length),
        offset_of!(OxyOSNetAnswer, bytes),
        offset_of!(OxyOSNetAnswer, length),
        offset_of!(OxyOSNetAnswer, progress),
        offset_of!(OxyOSNetAnswer, total),
        offset_of!(OxyOSNetAnswer, content_type),
        offset_of!(OxyOSNetAnswer, content_type_length),
        offset_of!(OxyOSNetAnswer, error),
        offset_of!(OxyOSNetAnswer, error_length),
        offset_of!(OxyOSNetAnswer, timed_out),
    ];
    assert_eq!(
        c_offsets(&answer),
        (rust.to_vec(), size_of::<OxyOSNetAnswer>())
    );
}

/// The `length` bytes at `data`, as text.
fn seen(data: *const c_char, length: usize) -> String {
    if data.is_null() {
        return String::new();
    }
    // SAFETY: an answer's pointers are valid for their lengths until the next answer.
    let bytes = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), length) };
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn a_request_through_the_c_abi_answers_its_response_data_and_end() {
    use net::*;
    use std::io::{BufRead, BufReader, Read as _, Write as _};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let (mut line, mut length, mut custom) = (String::new(), 0, String::new());
        reader.read_line(&mut line).unwrap();
        loop {
            let mut header = String::new();
            reader.read_line(&mut header).unwrap();
            let header = header.trim_end();
            if header.is_empty() {
                break;
            }
            let (name, value) = header.split_once(':').unwrap();
            match name.to_ascii_lowercase().as_str() {
                "content-length" => length = value.trim().parse().unwrap(),
                "x-oxy" => custom = value.trim().to_string(),
                _ => {}
            }
        }
        let mut body = vec![0u8; length];
        reader.read_exact(&mut body).unwrap();
        let text = format!(
            "{} {custom} {}",
            line.trim(),
            String::from_utf8_lossy(&body)
        );
        let mut stream = stream;
        write!(
            stream,
            "HTTP/1.1 201 Created\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
            text.len()
        )
        .unwrap();
    });

    let network = oxyos_net_new();
    assert!(!network.is_null());
    let (method, url, body) = ("PUT", format!("http://{address}/thing"), "ho");
    // The body's second half is a range of a file, handed over by its descriptor.
    let path = std::env::temp_dir().join(format!("oxyos-ffi-part-{}", std::process::id()));
    std::fs::write(&path, "--la--").unwrap();
    let file = std::fs::File::open(&path).unwrap();
    let parts = [
        OxyOSNetPart {
            bytes: body.as_ptr(),
            length: 2,
            fd: -1,
            offset: 0,
        },
        OxyOSNetPart {
            bytes: std::ptr::null(),
            length: 2,
            fd: std::os::fd::IntoRawFd::into_raw_fd(file),
            offset: 2,
        },
    ];
    std::fs::remove_file(&path).unwrap();
    let (name, value) = ("X-Oxy", "yes");
    let header = OxyOSNetHeader {
        name: name.as_ptr().cast(),
        name_length: name.len(),
        value: value.as_ptr().cast(),
        value_length: value.len(),
    };
    let request = OxyOSNetRequest {
        id: 7,
        method: method.as_ptr().cast(),
        method_length: method.len(),
        url: url.as_ptr().cast(),
        url_length: url.len(),
        headers: &header,
        header_count: 1,
        body: std::ptr::null(),
        body_length: 0,
        timeout_ms: 5000,
        has_body: 1,
        with_credentials: 1,
        incremental: 0,
        upload_progress: 1,
        parts: parts.as_ptr(),
        part_count: parts.len(),
    };
    // SAFETY: a live network, and a request whose pointers outlive the call.
    assert_eq!(unsafe { oxyos_net_send(network, &request) }, 0);

    let (mut got, mut sent) = (Vec::new(), Vec::new());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        let mut answer = OxyOSNetAnswer::default();
        // SAFETY: a live network and a writable answer.
        match unsafe { oxyos_net_answer(network, &mut answer) } {
            OXYOS_NET_ANSWER => {
                assert_eq!(answer.id, 7);
                let headers = if answer.header_count == 0 {
                    Vec::new()
                } else {
                    // SAFETY: valid for header_count until the next answer.
                    unsafe { std::slice::from_raw_parts(answer.headers, answer.header_count) }
                        .iter()
                        .map(|h| (seen(h.name, h.name_length), seen(h.value, h.value_length)))
                        .collect()
                };
                if answer.kind == OXYOS_NET_SENT {
                    sent.push((answer.progress, answer.total));
                    continue;
                }
                got.push((
                    answer.kind,
                    answer.status,
                    headers,
                    seen(answer.bytes.cast(), answer.length),
                    seen(answer.content_type, answer.content_type_length),
                    answer.error.is_null(),
                ));
                if answer.kind == OXYOS_NET_COMPLETE {
                    break;
                }
            }
            OXYOS_NET_EMPTY => std::thread::sleep(std::time::Duration::from_millis(5)),
            other => panic!("answer failed: {other}"),
        }
    }
    // SAFETY: a network from oxyos_net_new, freed once.
    unsafe { oxyos_net_free(network) };

    assert_eq!(sent.last(), Some(&(4, 4)), "{sent:?}");
    assert_eq!(got.len(), 3, "{got:?}");
    assert_eq!((got[0].0, got[0].1), (OXYOS_NET_RESPONSE, 201));
    assert!(got[0]
        .2
        .contains(&("content-type".to_string(), "text/plain".to_string())));
    assert_eq!(got[1].0, OXYOS_NET_DATA);
    assert_eq!(got[1].3, "PUT /thing HTTP/1.1 yes hola");
    assert_eq!(got[1].4, "text/plain");
    assert_eq!((got[2].0, got[2].5), (OXYOS_NET_COMPLETE, true));
}

#[test]
fn a_malformed_request_is_refused_and_sends_nothing() {
    use net::*;
    let network = oxyos_net_new();
    let bad = [0xff_u8, 0xfe];
    let url = "http://127.0.0.1:9/";
    let request = OxyOSNetRequest {
        id: 1,
        method: bad.as_ptr().cast(),
        method_length: bad.len(),
        url: url.as_ptr().cast(),
        url_length: url.len(),
        headers: std::ptr::null(),
        header_count: 0,
        body: std::ptr::null(),
        body_length: 0,
        timeout_ms: 0,
        has_body: 0,
        with_credentials: 1,
        incremental: 0,
        upload_progress: 0,
        parts: std::ptr::null(),
        part_count: 0,
    };
    // SAFETY: a live network, and a request whose pointers outlive the call.
    assert_eq!(unsafe { oxyos_net_send(network, &request) }, OXYOS_ERROR);
    // SAFETY: oxyos_last_error is NUL-terminated.
    let error = unsafe { CStr::from_ptr(oxyos_last_error()) };
    assert_eq!(error.to_str().unwrap(), "method is not UTF-8");
    let mut answer = OxyOSNetAnswer::default();
    // SAFETY: a live network and a writable answer.
    let answered = unsafe { oxyos_net_answer(network, &mut answer) };
    assert_eq!(answered, OXYOS_NET_EMPTY);
    // SAFETY: a network from oxyos_net_new, freed once.
    unsafe { oxyos_net_free(network) };
}

#[test]
fn the_header_matches_the_image_abi() {
    use images::*;
    assert_eq!(define("OXYOS_IMAGE_ANSWER"), i64::from(OXYOS_IMAGE_ANSWER));
    assert_eq!(define("OXYOS_IMAGE_EMPTY"), i64::from(OXYOS_IMAGE_EMPTY));
    assert_eq!(
        define("OXYOS_IMAGE_FIT_WITHIN"),
        i64::from(OXYOS_IMAGE_FIT_WITHIN)
    );
    assert_eq!(
        define("OXYOS_IMAGE_FIT_COVERING"),
        i64::from(OXYOS_IMAGE_FIT_COVERING)
    );
    let answer = fields("OxyOSImageAnswer");
    assert_eq!(
        names(&answer),
        [
            "id",
            "width",
            "height",
            "natural_width",
            "natural_height",
            "pixels",
            "length",
            "error",
            "error_length"
        ]
    );
    let rust = [
        offset_of!(OxyOSImageAnswer, id),
        offset_of!(OxyOSImageAnswer, width),
        offset_of!(OxyOSImageAnswer, height),
        offset_of!(OxyOSImageAnswer, natural_width),
        offset_of!(OxyOSImageAnswer, natural_height),
        offset_of!(OxyOSImageAnswer, pixels),
        offset_of!(OxyOSImageAnswer, length),
        offset_of!(OxyOSImageAnswer, error),
        offset_of!(OxyOSImageAnswer, error_length),
    ];
    assert_eq!(
        c_offsets(&answer),
        (rust.to_vec(), size_of::<OxyOSImageAnswer>())
    );
}

#[test]
fn an_image_loads_through_the_c_abi() {
    use images::*;
    let wallpaper = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/wallpapers/solar-gradients/frames/frame-08.jpg"
    );
    let missing = "/nonexistent/oxy.png";
    let (mut width, mut height) = (0, 0);
    // SAFETY: the path is valid for its length; the sizes are writable.
    let sized = unsafe {
        oxyos_image_size(
            wallpaper.as_ptr().cast(),
            wallpaper.len(),
            &mut width,
            &mut height,
        )
    };
    assert_eq!((sized, width, height), (0, 5120, 2880));
    let images = oxyos_images_new();
    assert!(!images.is_null());
    // SAFETY: a live loader, and paths valid for their lengths.
    unsafe {
        assert_eq!(
            oxyos_images_load(
                images,
                1,
                wallpaper.as_ptr().cast(),
                wallpaper.len(),
                640,
                640,
                OXYOS_IMAGE_FIT_COVERING
            ),
            0
        );
        assert_eq!(
            oxyos_images_load(
                images,
                2,
                missing.as_ptr().cast(),
                missing.len(),
                0,
                0,
                OXYOS_IMAGE_FIT_WITHIN
            ),
            0
        );
    }
    let mut got = std::collections::BTreeMap::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while got.len() < 2 && std::time::Instant::now() < deadline {
        let mut answer = OxyOSImageAnswer::default();
        // SAFETY: a live loader and a writable answer.
        match unsafe { oxyos_images_answer(images, &mut answer) } {
            OXYOS_IMAGE_ANSWER => {
                let error = if answer.error.is_null() {
                    None
                } else {
                    // SAFETY: valid for error_length until the next answer.
                    let bytes = unsafe {
                        std::slice::from_raw_parts(answer.error.cast::<u8>(), answer.error_length)
                    };
                    Some(String::from_utf8_lossy(bytes).into_owned())
                };
                got.insert(
                    answer.id,
                    (
                        answer.width,
                        answer.height,
                        answer.length,
                        answer.pixels.is_null(),
                        error,
                    ),
                );
            }
            OXYOS_IMAGE_EMPTY => std::thread::sleep(std::time::Duration::from_millis(5)),
            other => panic!("answer failed: {other}"),
        }
    }
    // SAFETY: a loader from oxyos_images_new, freed once.
    unsafe { oxyos_images_free(images) };
    assert_eq!(got[&1], (1138, 640, 1138 * 640 * 4, false, None));
    let (_, _, _, no_pixels, error) = &got[&2];
    assert!(
        *no_pixels
            && error
                .as_deref()
                .is_some_and(|error| error.starts_with("cannot read the image"))
    );
}

#[test]
fn the_header_matches_the_text_input_abi() {
    assert_eq!(
        define("OXYOS_EVENT_TEXT_INPUT"),
        i64::from(OXYOS_EVENT_TEXT_INPUT)
    );
    let done = Event::TextInputDone {
        surface: 1,
        serial: 0,
        commit: 0,
    };
    assert_eq!(i64::from(done.opcode()), define("OXYOS_EVENT_TEXT_INPUT"));
    // Every purpose and style, both ways; the hints as the protocol's bits.
    let named = |prefix: &str| -> Vec<(String, i64)> {
        HEADER
            .lines()
            .filter_map(|line| line.strip_prefix(&format!("#define {prefix}")))
            .map(|rest| {
                let (name, value) = rest.split_once(' ').unwrap();
                (name.to_string(), value.parse().unwrap())
            })
            .collect()
    };
    let screaming = |debug: String| -> String {
        debug
            .chars()
            .enumerate()
            .flat_map(|(i, c)| {
                [
                    (i > 0 && c.is_ascii_uppercase()).then_some('_'),
                    Some(c.to_ascii_uppercase()),
                ]
            })
            .flatten()
            .collect()
    };
    let purposes = named("OXYOS_PURPOSE_");
    for (name, value) in &purposes {
        let purpose = TextPurpose::from_raw(*value as u8).unwrap_or_else(|| panic!("{name}"));
        assert_eq!(&screaming(format!("{purpose:?}")), name);
    }
    assert!(TextPurpose::from_raw(purposes.len() as u8).is_none());
    let styles = named("OXYOS_PREEDIT_");
    for (name, value) in &styles {
        let style = oxyos_protocol::PreeditStyle::from_raw(*value as u8)
            .unwrap_or_else(|| panic!("{name}"));
        assert_eq!(&screaming(format!("{style:?}")), name);
    }
    for (name, bit) in [
        ("OXYOS_HINT_NO_PREDICTION", TextHints::NO_PREDICTION),
        ("OXYOS_HINT_NO_CAPITALIZATION", TextHints::NO_CAPITALIZATION),
        ("OXYOS_HINT_SENSITIVE", TextHints::SENSITIVE),
        ("OXYOS_HINT_MULTILINE", TextHints::MULTILINE),
    ] {
        assert_eq!(define(name), i64::from(bit), "{name}");
    }

    let field = fields("OxyOSTextField");
    assert_eq!(
        names(&field),
        [
            "purpose",
            "hints",
            "text",
            "length",
            "anchor",
            "cursor",
            "has_caret",
            "caret_x",
            "caret_y",
            "caret_width",
            "caret_height"
        ]
    );
    let rust = [
        offset_of!(OxyOSTextField, purpose),
        offset_of!(OxyOSTextField, hints),
        offset_of!(OxyOSTextField, text),
        offset_of!(OxyOSTextField, length),
        offset_of!(OxyOSTextField, anchor),
        offset_of!(OxyOSTextField, cursor),
        offset_of!(OxyOSTextField, has_caret),
        offset_of!(OxyOSTextField, caret_x),
        offset_of!(OxyOSTextField, caret_y),
        offset_of!(OxyOSTextField, caret_width),
        offset_of!(OxyOSTextField, caret_height),
    ];
    assert_eq!(
        c_offsets(&field),
        (rust.to_vec(), size_of::<OxyOSTextField>())
    );

    let segment = fields("OxyOSPreeditSegment");
    assert_eq!(names(&segment), ["begin", "end", "style"]);
    let rust = [
        offset_of!(OxyOSPreeditSegment, begin),
        offset_of!(OxyOSPreeditSegment, end),
        offset_of!(OxyOSPreeditSegment, style),
    ];
    assert_eq!(
        c_offsets(&segment),
        (rust.to_vec(), size_of::<OxyOSPreeditSegment>())
    );

    let transaction = fields("OxyOSTextTransaction");
    assert_eq!(
        names(&transaction),
        [
            "surface",
            "delete_before",
            "delete_after",
            "cursor_begin",
            "cursor_end",
            "commit_serial",
            "serial",
            "commit",
            "commit_length",
            "preedit",
            "preedit_length",
            "segments",
            "segment_count"
        ]
    );
    let rust = [
        offset_of!(OxyOSTextTransaction, surface),
        offset_of!(OxyOSTextTransaction, delete_before),
        offset_of!(OxyOSTextTransaction, delete_after),
        offset_of!(OxyOSTextTransaction, cursor_begin),
        offset_of!(OxyOSTextTransaction, cursor_end),
        offset_of!(OxyOSTextTransaction, commit_serial),
        offset_of!(OxyOSTextTransaction, serial),
        offset_of!(OxyOSTextTransaction, commit),
        offset_of!(OxyOSTextTransaction, commit_length),
        offset_of!(OxyOSTextTransaction, preedit),
        offset_of!(OxyOSTextTransaction, preedit_length),
        offset_of!(OxyOSTextTransaction, segments),
        offset_of!(OxyOSTextTransaction, segment_count),
    ];
    assert_eq!(
        c_offsets(&transaction),
        (rust.to_vec(), size_of::<OxyOSTextTransaction>())
    );
}

#[test]
fn text_input_through_the_c_abi() {
    let directory =
        std::env::temp_dir().join(format!("oxyos-client-ffi-text-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("s");
    let listener = Listener::bind(&socket).unwrap();
    let compositor = std::thread::spawn(move || {
        let server = listener.accept().unwrap();
        let mut protocol = ServerConnection::new();
        let (hello, _) = server.recv_request().unwrap();
        server
            .send_event(&protocol.receive(&hello).unwrap().unwrap(), None)
            .unwrap();
        let mut requests = Vec::new();
        loop {
            let (request, _) = server.recv_request().unwrap();
            let done = matches!(request, Request::TextInputCommit { .. });
            requests.push(request);
            if done {
                break;
            }
        }
        let chunk = |text| TextChunk::new(text).unwrap();
        for event in [
            Event::TextInputDelete {
                surface: 1,
                before: 1,
                after: 0,
            },
            Event::TextInputCommitText {
                surface: 1,
                text: chunk("日本"),
            },
            Event::TextInputPreeditText {
                surface: 1,
                text: chunk("ご"),
            },
            Event::TextInputPreeditCursor {
                surface: 1,
                begin: 3,
                end: 3,
            },
            Event::TextInputPreeditSegment {
                surface: 1,
                begin: 0,
                end: 3,
                style: oxyos_protocol::PreeditStyle::Composing,
            },
            Event::TextInputDone {
                surface: 1,
                serial: 40,
                commit: 1,
            },
            Event::Focus {
                surface: 1,
                focused: false,
            },
        ] {
            server.send_event(&event, None).unwrap();
        }
        requests
    });

    let path = CString::new(socket.to_str().unwrap()).unwrap();
    let text = "nate@";
    let field = OxyOSTextField {
        purpose: TextPurpose::Email as u8,
        hints: TextHints::NO_CAPITALIZATION,
        text: text.as_ptr().cast(),
        length: text.len(),
        anchor: 5,
        cursor: 5,
        has_caret: 1,
        caret_x: 40,
        caret_y: 8,
        caret_width: 2,
        caret_height: 18,
    };
    // SAFETY: the client pointer comes from oxyos_client_connect and is closed once; the field
    // and its text outlive the calls.
    unsafe {
        let client = oxyos_client_connect(path.as_ptr());
        assert!(!client.is_null());
        // A selection inside a character, or an unknown purpose, is refused before sending.
        let inside = OxyOSTextField {
            text: "ñ".as_ptr().cast(),
            length: 2,
            anchor: 1,
            cursor: 1,
            ..field
        };
        assert_eq!(oxyos_client_text_input(client, 1, &inside, 0), -1);
        let unknown = OxyOSTextField {
            purpose: 200,
            ..field
        };
        assert_eq!(oxyos_client_text_input(client, 1, &unknown, 0), -1);
        assert_eq!(oxyos_client_text_input(client, 1, &field, 0), 1);
        let mut event = OxyOSEvent::default();
        let mut transaction: OxyOSTextTransaction = std::mem::zeroed();
        let mut next = || loop {
            match oxyos_client_poll(client, &mut event) {
                OXYOS_POLL_EVENT => break event,
                OXYOS_POLL_EMPTY => std::thread::yield_now(),
                other => panic!("poll returned {other}"),
            }
        };
        // The parts are not events of their own: the first is the completed transaction.
        let done = next();
        assert_eq!(
            (done.kind, done.surface, done.serial, done.detail),
            (OXYOS_EVENT_TEXT_INPUT, 1, 40, 1)
        );
        assert_eq!(oxyos_client_text_transaction(client, &mut transaction), 1);
        assert_eq!(
            (
                transaction.delete_before,
                transaction.delete_after,
                seen(transaction.commit, transaction.commit_length),
                seen(transaction.preedit, transaction.preedit_length),
                (transaction.cursor_begin, transaction.cursor_end),
                transaction.commit_serial,
            ),
            (1, 0, "日本".to_string(), "ご".to_string(), (3, 3), 1)
        );
        let segments = std::slice::from_raw_parts(transaction.segments, transaction.segment_count);
        assert_eq!(
            segments,
            [OxyOSPreeditSegment {
                begin: 0,
                end: 3,
                style: oxyos_protocol::PreeditStyle::Composing as u8,
            }]
        );
        // The next event drops it.
        assert_eq!(next().kind, OXYOS_EVENT_FOCUS);
        assert_eq!(oxyos_client_text_transaction(client, &mut transaction), 0);
        let requests = compositor.join().unwrap();
        assert_eq!(
            requests.first(),
            Some(&Request::TextInputEnable { surface: 1 })
        );
        assert!(requests.contains(&Request::TextInputPurpose {
            surface: 1,
            purpose: TextPurpose::Email,
            hints: TextHints::new(TextHints::NO_CAPITALIZATION).unwrap(),
        }));
        assert!(requests.contains(&Request::TextInputCaret {
            surface: 1,
            rect: Rect::new(40, 8, 2, 18),
        }));
        oxyos_client_close(client);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn the_header_matches_the_shell_surface_abi() {
    for (name, value) in [
        ("OXYOS_SHELL_BACKGROUND", ShellKind::Background as i64),
        ("OXYOS_SHELL_PANEL", ShellKind::Panel as i64),
        ("OXYOS_SHELL_OVERLAY", ShellKind::Overlay as i64),
        ("OXYOS_SHELL_NOTIFICATION", ShellKind::Notification as i64),
        ("OXYOS_SHELL_BACKDROP", ShellKind::Backdrop as i64),
        ("OXYOS_EDGE_NONE", PanelEdge::None as i64),
        ("OXYOS_EDGE_TOP", PanelEdge::Top as i64),
        ("OXYOS_EDGE_BOTTOM", PanelEdge::Bottom as i64),
        ("OXYOS_EDGE_LEFT", PanelEdge::Left as i64),
        ("OXYOS_EDGE_RIGHT", PanelEdge::Right as i64),
    ] {
        assert_eq!(define(name), value, "{name}");
    }
}

#[test]
fn shell_surfaces_and_input_regions_through_the_c_abi() {
    let directory =
        std::env::temp_dir().join(format!("oxyos-client-ffi-shell-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("s");
    let listener = Listener::bind(&socket).unwrap();
    let compositor = std::thread::spawn(move || {
        let server = listener.accept().unwrap();
        let mut protocol = ServerConnection::new();
        let (hello, _) = server.recv_request().unwrap();
        server
            .send_event(&protocol.receive(&hello).unwrap().unwrap(), None)
            .unwrap();
        (0..2)
            .map(|_| server.recv_request().unwrap().0)
            .collect::<Vec<_>>()
    });
    let path = CString::new(socket.to_str().unwrap()).unwrap();
    // SAFETY: the client pointer comes from oxyos_client_connect and is closed once.
    unsafe {
        let client = oxyos_client_connect(path.as_ptr());
        assert!(!client.is_null());
        // Unknown kinds and edges are refused before anything is sent.
        assert_eq!(
            oxyos_client_create_shell_surface(client, 2, 9, 1, 1, 100, 40, 40),
            OXYOS_ERROR
        );
        assert_eq!(
            oxyos_client_create_shell_surface(client, 2, 2, 9, 1, 100, 40, 40),
            OXYOS_ERROR
        );
        assert_eq!(
            oxyos_client_create_shell_surface(client, 2, 2, 1, 1, 100, 40, 40),
            0
        );
        assert_eq!(oxyos_client_set_input_region(client, 2, 0, 8, 100, 32), 0);
        let requests = compositor.join().unwrap();
        assert_eq!(
            requests,
            [
                Request::CreateShellSurface {
                    surface: 2,
                    kind: ShellKind::Panel,
                    edge: PanelEdge::Top,
                    output: 1,
                    size: Size::new(100, 40),
                    exclusive: 40,
                },
                Request::SetInputRegion {
                    surface: 2,
                    region: Rect::new(0, 8, 100, 32),
                },
            ]
        );
        oxyos_client_close(client);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn the_header_matches_the_window_management_abi() {
    for (name, value) in [
        (
            "OXYOS_EVENT_WINDOW_STATE",
            i64::from(OXYOS_EVENT_WINDOW_STATE),
        ),
        ("OXYOS_EVENT_WINDOW_TITLE", OXYOS_EVENT_WINDOW_TITLE.into()),
        (
            "OXYOS_EVENT_WINDOW_APP_ID",
            OXYOS_EVENT_WINDOW_APP_ID.into(),
        ),
        (
            "OXYOS_EVENT_WINDOW_CLOSED",
            OXYOS_EVENT_WINDOW_CLOSED.into(),
        ),
        ("OXYOS_EVENT_WORKSPACES", OXYOS_EVENT_WORKSPACES.into()),
        ("OXYOS_EVENT_OUTPUT", OXYOS_EVENT_OUTPUT.into()),
        (
            "OXYOS_EVENT_OUTPUT_REMOVED",
            OXYOS_EVENT_OUTPUT_REMOVED.into(),
        ),
        ("OXYOS_EVENT_REFRESH", OXYOS_EVENT_REFRESH.into()),
        (
            "OXYOS_EVENT_CLOSE_REQUESTED",
            OXYOS_EVENT_CLOSE_REQUESTED.into(),
        ),
        ("OXYOS_EVENT_PINCH", OXYOS_EVENT_PINCH.into()),
        ("OXYOS_PINCH_BEGIN", OXYOS_PINCH_BEGIN.into()),
        ("OXYOS_PINCH_UPDATE", OXYOS_PINCH_UPDATE.into()),
        ("OXYOS_PINCH_END", OXYOS_PINCH_END.into()),
        ("OXYOS_PINCH_CANCEL", OXYOS_PINCH_CANCEL.into()),
        ("OXYOS_SCROLL_WHEEL", OXYOS_SCROLL_WHEEL.into()),
        ("OXYOS_SCROLL_FINGER", OXYOS_SCROLL_FINGER.into()),
        ("OXYOS_SCROLL_CONTINUOUS", OXYOS_SCROLL_CONTINUOUS.into()),
        ("OXYOS_WINDOW_MINIMIZED", OXYOS_WINDOW_MINIMIZED.into()),
        ("OXYOS_WINDOW_FOCUSED", OXYOS_WINDOW_FOCUSED.into()),
        ("OXYOS_WINDOW_LAUNCHED", OXYOS_WINDOW_LAUNCHED.into()),
        ("OXYOS_MANAGE_FOCUS", OXYOS_MANAGE_FOCUS.into()),
        ("OXYOS_MANAGE_CLOSE", OXYOS_MANAGE_CLOSE.into()),
        ("OXYOS_MANAGE_MINIMIZE", OXYOS_MANAGE_MINIMIZE.into()),
        ("OXYOS_MANAGE_MOVE", OXYOS_MANAGE_MOVE.into()),
        (
            "OXYOS_MANAGE_ACTIVATE_WORKSPACE",
            OXYOS_MANAGE_ACTIVATE_WORKSPACE.into(),
        ),
        (
            "OXYOS_MANAGE_ADD_WORKSPACE",
            OXYOS_MANAGE_ADD_WORKSPACE.into(),
        ),
        (
            "OXYOS_MANAGE_REMOVE_WORKSPACE",
            OXYOS_MANAGE_REMOVE_WORKSPACE.into(),
        ),
    ] {
        assert_eq!(define(name), value, "{name}");
    }
    // The event kinds are the protocol's opcodes.
    let state = Event::WindowState {
        window: 1,
        workspace: 0,
        minimized: false,
        focused: false,
        launched: false,
        responding: true,
    };
    assert_eq!(u32::from(state.opcode()), OXYOS_EVENT_WINDOW_STATE);
    assert_eq!(
        u32::from(Event::CloseRequested { surface: 1 }.opcode()),
        OXYOS_EVENT_CLOSE_REQUESTED
    );
    let pinch = Event::Pinch {
        surface: 1,
        serial: 2,
        phase: GesturePhase::Cancel,
        x: 0,
        y: 0,
        scale_65536: 0,
        rotation_256: 0,
        time_ns: 5_000_000_123,
    };
    assert_eq!(u32::from(pinch.opcode()), OXYOS_EVENT_PINCH);
    assert_eq!(GesturePhase::Begin as u32, OXYOS_PINCH_BEGIN);
    assert_eq!(GesturePhase::Update as u32, OXYOS_PINCH_UPDATE);
    assert_eq!(GesturePhase::End as u32, OXYOS_PINCH_END);
    assert_eq!(GesturePhase::Cancel as u32, OXYOS_PINCH_CANCEL);
    assert_eq!(ScrollSource::Wheel as u32, OXYOS_SCROLL_WHEEL);
    assert_eq!(ScrollSource::Finger as u32, OXYOS_SCROLL_FINGER);
    assert_eq!(ScrollSource::Continuous as u32, OXYOS_SCROLL_CONTINUOUS);
}

#[test]
fn a_pinch_converts_field_by_field() {
    let event = convert(&Event::Pinch {
        surface: 3,
        serial: 9,
        phase: GesturePhase::Update,
        x: 2560,
        y: -512,
        scale_65536: 98304,
        rotation_256: -2560,
        time_ns: 5_000_000_123,
    })
    .unwrap();
    assert_eq!(
        (
            event.kind,
            event.surface,
            event.serial,
            event.x,
            event.y,
            event.detail,
            event.scale_65536,
            event.rotation_256
        ),
        (
            OXYOS_EVENT_PINCH,
            3,
            9,
            2560,
            -512,
            OXYOS_PINCH_UPDATE,
            98304,
            -2560
        )
    );
}

#[test]
fn resting_fingers_convert_field_by_field() {
    let hold = Event::Hold {
        surface: 3,
        serial: 9,
        phase: GesturePhase::Begin,
        fingers: 2,
        time_ns: 5_000_000_123,
    };
    assert_eq!(u32::from(hold.opcode()), OXYOS_EVENT_HOLD);
    let event = convert(&hold).unwrap();
    assert_eq!(
        (
            event.kind,
            event.surface,
            event.serial,
            event.detail,
            event.code,
            event.time_ns
        ),
        (OXYOS_EVENT_HOLD, 3, 9, OXYOS_PINCH_BEGIN, 2, 5_000_000_123)
    );
}

#[test]
fn window_events_convert_field_by_field() {
    let state = convert(&Event::WindowState {
        window: 7,
        workspace: 2,
        minimized: true,
        focused: true,
        launched: false,
        responding: false,
    })
    .unwrap();
    assert_eq!(
        (state.kind, state.code, state.detail, state.flag),
        (
            OXYOS_EVENT_WINDOW_STATE,
            7,
            2,
            OXYOS_WINDOW_MINIMIZED | OXYOS_WINDOW_FOCUSED
        )
    );
    let title = convert(&Event::WindowTitle {
        window: 7,
        index: 1,
        last: true,
        text: oxyos_protocol::PreeditText::new("entrada").unwrap(),
    })
    .unwrap();
    assert_eq!(
        (title.kind, title.code, title.detail, title.flag),
        (OXYOS_EVENT_WINDOW_TITLE, 7, 1, 1)
    );
    assert_eq!(&title.text[..8], b"entrada\0");
    let spaces = convert(&Event::Workspaces {
        count: 3,
        active: 1,
    })
    .unwrap();
    assert_eq!(
        (spaces.kind, spaces.detail, spaces.code),
        (OXYOS_EVENT_WORKSPACES, 3, 1)
    );
    let output = convert(&Event::Output {
        output: 2,
        rect: oxyos_protocol::Rect::new(1536, 0, 1707, 960),
        scale_120: 180,
        refresh_mhz: 59_951,
        primary: false,
    })
    .unwrap();
    assert_eq!(
        (
            output.kind,
            output.code,
            (output.x, output.y, output.width, output.height),
            output.scale_120,
            output.detail,
            output.flag
        ),
        (OXYOS_EVENT_OUTPUT, 2, (1536, 0, 1707, 960), 180, 59_951, 0)
    );
    let removed = convert(&Event::OutputRemoved { output: 2 }).unwrap();
    assert_eq!(
        (removed.kind, removed.code),
        (OXYOS_EVENT_OUTPUT_REMOVED, 2)
    );
    let refresh = convert(&Event::Refresh {
        surface: 1,
        refresh_mhz: 119_998,
    })
    .unwrap();
    assert_eq!(
        (refresh.kind, refresh.surface, refresh.detail),
        (OXYOS_EVENT_REFRESH, 1, 119_998)
    );
}

#[test]
fn naming_and_managing_windows_through_the_c_abi() {
    let directory =
        std::env::temp_dir().join(format!("oxyos-client-ffi-manage-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("s");
    let listener = Listener::bind(&socket).unwrap();
    let compositor = std::thread::spawn(move || {
        let server = listener.accept().unwrap();
        let mut protocol = ServerConnection::new();
        let (hello, _) = server.recv_request().unwrap();
        server
            .send_event(&protocol.receive(&hello).unwrap().unwrap(), None)
            .unwrap();
        (0..4)
            .map(|_| server.recv_request().unwrap().0)
            .collect::<Vec<_>>()
    });
    let path = CString::new(socket.to_str().unwrap()).unwrap();
    let title = "a".repeat(32) + "b";
    // SAFETY: the client pointer comes from oxyos_client_connect and is closed once; the title
    // outlives the call.
    unsafe {
        let client = oxyos_client_connect(path.as_ptr());
        assert!(!client.is_null());
        assert_eq!(
            oxyos_client_name_window(client, 1, 0, title.as_ptr().cast(), title.len()),
            0
        );
        assert_eq!(oxyos_client_manage(client, 99, 1, 0), OXYOS_ERROR);
        assert_eq!(oxyos_client_manage(client, OXYOS_MANAGE_MOVE, 7, 2), 0);
        assert_eq!(
            oxyos_client_manage(client, OXYOS_MANAGE_ADD_WORKSPACE, 0, 0),
            0
        );
        let requests = compositor.join().unwrap();
        let chunk = |text: &str| oxyos_protocol::PreeditText::new(text).unwrap();
        assert_eq!(
            requests,
            [
                Request::SetTitle {
                    surface: 1,
                    index: 0,
                    last: false,
                    text: chunk(&"a".repeat(32)),
                },
                Request::SetTitle {
                    surface: 1,
                    index: 1,
                    last: true,
                    text: chunk("b"),
                },
                Request::MoveWindowToWorkspace {
                    window: 7,
                    workspace: 2,
                },
                Request::AddWorkspace {},
            ]
        );
        oxyos_client_close(client);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn the_header_matches_the_gestures_abi() {
    use crate::gestures::*;
    for (name, value) in [
        ("OXYOS_GESTURE_DOWN", OXYOS_GESTURE_DOWN),
        ("OXYOS_GESTURE_POINTER_DOWN", OXYOS_GESTURE_POINTER_DOWN),
        ("OXYOS_GESTURE_MOVE", OXYOS_GESTURE_MOVE),
        ("OXYOS_GESTURE_POINTER_UP", OXYOS_GESTURE_POINTER_UP),
        ("OXYOS_GESTURE_UP", OXYOS_GESTURE_UP),
        ("OXYOS_GESTURE_CANCEL", OXYOS_GESTURE_CANCEL),
        ("OXYOS_GESTURE_NULL", OXYOS_GESTURE_NULL),
        ("OXYOS_GESTURE_BOOL", OXYOS_GESTURE_BOOL),
        ("OXYOS_GESTURE_NUMBER", OXYOS_GESTURE_NUMBER),
        ("OXYOS_GESTURE_TEXT", OXYOS_GESTURE_TEXT),
        ("OXYOS_GESTURE_LIST", OXYOS_GESTURE_LIST),
        ("OXYOS_GESTURE_STATE_CHANGE", OXYOS_GESTURE_STATE_CHANGE),
        ("OXYOS_GESTURE_UPDATE", OXYOS_GESTURE_UPDATE),
        ("OXYOS_GESTURE_INTERCEPT", OXYOS_GESTURE_INTERCEPT),
        (
            "OXYOS_GESTURE_CANCEL_RESPONDER",
            OXYOS_GESTURE_CANCEL_RESPONDER,
        ),
        ("OXYOS_GESTURE_TOUCH", OXYOS_GESTURE_TOUCH),
        ("OXYOS_GESTURE_TOUCH_DOWN", OXYOS_GESTURE_TOUCH_DOWN),
        ("OXYOS_GESTURE_TOUCH_MOVE", OXYOS_GESTURE_TOUCH_MOVE),
        ("OXYOS_GESTURE_TOUCH_UP", OXYOS_GESTURE_TOUCH_UP),
        (
            "OXYOS_GESTURE_TOUCH_CANCELLED",
            OXYOS_GESTURE_TOUCH_CANCELLED,
        ),
        ("OXYOS_GESTURE_HAS_POSITION", OXYOS_GESTURE_HAS_POSITION),
        (
            "OXYOS_GESTURE_HAS_TRANSLATION",
            OXYOS_GESTURE_HAS_TRANSLATION,
        ),
        ("OXYOS_GESTURE_HAS_VELOCITY", OXYOS_GESTURE_HAS_VELOCITY),
        ("OXYOS_GESTURE_HAS_DURATION", OXYOS_GESTURE_HAS_DURATION),
        (
            "OXYOS_GESTURE_HAS_POINTER_INSIDE",
            OXYOS_GESTURE_HAS_POINTER_INSIDE,
        ),
        ("OXYOS_GESTURE_HAS_SCALE", OXYOS_GESTURE_HAS_SCALE),
        ("OXYOS_GESTURE_HAS_ROTATION", OXYOS_GESTURE_HAS_ROTATION),
    ] {
        assert_eq!(define(name), i64::from(value), "{name}");
    }
    let value = fields("OxyOSGestureValue");
    assert_eq!(
        names(&value),
        ["key", "key_length", "kind", "list_set", "number", "list"]
    );
    let rust = [
        offset_of!(OxyOSGestureValue, key),
        offset_of!(OxyOSGestureValue, key_length),
        offset_of!(OxyOSGestureValue, kind),
        offset_of!(OxyOSGestureValue, list_set),
        offset_of!(OxyOSGestureValue, number),
        offset_of!(OxyOSGestureValue, list),
    ];
    assert_eq!(
        c_offsets(&value),
        (rust.to_vec(), size_of::<OxyOSGestureValue>())
    );
    let pointer = fields("OxyOSGesturePointer");
    assert_eq!(names(&pointer), ["id", "x", "y"]);
    let rust = [
        offset_of!(OxyOSGesturePointer, id),
        offset_of!(OxyOSGesturePointer, x),
        offset_of!(OxyOSGesturePointer, y),
    ];
    assert_eq!(
        c_offsets(&pointer),
        (rust.to_vec(), size_of::<OxyOSGesturePointer>())
    );
    let frame = fields("OxyOSGestureFrame");
    assert_eq!(names(&frame), ["x", "y", "width", "height"]);
    let touch = fields("OxyOSGestureTouch");
    assert_eq!(names(&touch), ["id", "x", "y", "absolute_x", "absolute_y"]);
    assert_eq!(c_offsets(&touch).1, size_of::<OxyOSGestureTouch>());
    let output = fields("OxyOSGestureOutput");
    assert_eq!(
        names(&output),
        [
            "kind",
            "state",
            "old_state",
            "pointer_type",
            "fields",
            "pointer_inside",
            "event_type",
            "tag",
            "view",
            "number_of_pointers",
            "x",
            "y",
            "absolute_x",
            "absolute_y",
            "translation_x",
            "translation_y",
            "velocity_x",
            "velocity_y",
            "duration",
            "scale",
            "focal_x",
            "focal_y",
            "rotation",
            "anchor_x",
            "anchor_y",
            "rate"
        ]
    );
    let rust = [
        offset_of!(OxyOSGestureOutput, kind),
        offset_of!(OxyOSGestureOutput, state),
        offset_of!(OxyOSGestureOutput, old_state),
        offset_of!(OxyOSGestureOutput, pointer_type),
        offset_of!(OxyOSGestureOutput, fields),
        offset_of!(OxyOSGestureOutput, pointer_inside),
        offset_of!(OxyOSGestureOutput, event_type),
        offset_of!(OxyOSGestureOutput, tag),
        offset_of!(OxyOSGestureOutput, view),
        offset_of!(OxyOSGestureOutput, number_of_pointers),
        offset_of!(OxyOSGestureOutput, x),
        offset_of!(OxyOSGestureOutput, y),
        offset_of!(OxyOSGestureOutput, absolute_x),
        offset_of!(OxyOSGestureOutput, absolute_y),
        offset_of!(OxyOSGestureOutput, translation_x),
        offset_of!(OxyOSGestureOutput, translation_y),
        offset_of!(OxyOSGestureOutput, velocity_x),
        offset_of!(OxyOSGestureOutput, velocity_y),
        offset_of!(OxyOSGestureOutput, duration),
        offset_of!(OxyOSGestureOutput, scale),
        offset_of!(OxyOSGestureOutput, focal_x),
        offset_of!(OxyOSGestureOutput, focal_y),
        offset_of!(OxyOSGestureOutput, rotation),
        offset_of!(OxyOSGestureOutput, anchor_x),
        offset_of!(OxyOSGestureOutput, anchor_y),
        offset_of!(OxyOSGestureOutput, rate),
    ];
    assert_eq!(
        c_offsets(&output),
        (rust.to_vec(), size_of::<OxyOSGestureOutput>())
    );
}

#[test]
fn a_tap_is_recognized_through_the_c_abi() {
    use crate::gestures::*;
    use std::ffi::c_void;

    #[allow(unsafe_code)]
    unsafe extern "C" fn frame(_: *mut c_void, view: i32, out: *mut OxyOSGestureFrame) -> u8 {
        if view != 10 {
            return 0;
        }
        // SAFETY: the orchestrator passes a writable frame.
        unsafe {
            *out = OxyOSGestureFrame {
                x: 50.0,
                y: 50.0,
                width: 100.0,
                height: 100.0,
            }
        };
        1
    }
    let name = "TapGestureHandler";
    let key = "maxDurationMs";
    let refused_key = "mouseButton";
    let touches_key = "needsPointerData";
    let setting = |key: &str, kind, number| OxyOSGestureValue {
        key: key.as_ptr().cast(),
        key_length: key.len(),
        kind,
        list_set: 0,
        number,
        list: [0.0; 6],
    };
    let config = [
        setting(key, OXYOS_GESTURE_NUMBER, 300.0),
        setting(refused_key, OXYOS_GESTURE_NUMBER, 2.0),
        setting(touches_key, OXYOS_GESTURE_BOOL, 1.0),
    ];
    let mut refused = [9u8; 3];
    let hits = [10, 1];
    let mut out = OxyOSGestureOutput::default();
    let gestures = oxyos_gestures_new();
    // SAFETY: the orchestrator is live until freed below; every pointer passed outlives its call
    // with the counts given.
    unsafe {
        assert_eq!(
            oxyos_gestures_create(
                gestures,
                name.as_ptr().cast(),
                name.len(),
                7,
                config.as_ptr(),
                config.len(),
                refused.as_mut_ptr()
            ),
            0
        );
        assert_eq!(refused, [0, 1, 0]);
        let hover = "HoverGestureHandler";
        assert_eq!(
            oxyos_gestures_create(
                gestures,
                hover.as_ptr().cast(),
                hover.len(),
                8,
                config.as_ptr(),
                0,
                std::ptr::null_mut()
            ),
            OXYOS_ERROR
        );
        assert_eq!(oxyos_gestures_attach(gestures, 7, 10), 0);
        let mut pointer = [OxyOSGesturePointer {
            id: 0,
            x: 60.0,
            y: 70.0,
        }];
        let touch = |action, pointer: &[OxyOSGesturePointer], time| {
            oxyos_gestures_touch(
                gestures,
                action,
                0,
                pointer.as_ptr(),
                1,
                time,
                time,
                2,
                hits.as_ptr(),
                hits.len(),
                Some(frame),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(touch(OXYOS_GESTURE_DOWN, &pointer, 1000), 0);
        assert_eq!(oxyos_gestures_next_deadline(gestures), 1300);
        // Its touch event first: the pointer down, in the view and in the root.
        assert_eq!(oxyos_gestures_output(gestures, &mut out), 1);
        assert_eq!(
            (out.kind, out.tag, out.event_type, out.number_of_pointers),
            (OXYOS_GESTURE_TOUCH, 7, OXYOS_GESTURE_TOUCH_DOWN, 1)
        );
        let mut points = [OxyOSGestureTouch::default(); 2];
        assert_eq!(
            oxyos_gestures_touches(gestures, 0, points.as_mut_ptr(), 2),
            1
        );
        let first = points[0];
        assert_eq!(
            (
                first.id,
                first.x,
                first.y,
                first.absolute_x,
                first.absolute_y
            ),
            (0, 10.0, 20.0, 60.0, 70.0)
        );
        assert_eq!(
            oxyos_gestures_touches(gestures, 1, std::ptr::null_mut(), 0),
            1
        );
        assert_eq!(oxyos_gestures_output(gestures, &mut out), 1);
        assert_eq!(
            (out.kind, out.tag, out.view, out.old_state, out.state),
            (OXYOS_GESTURE_STATE_CHANGE, 7, 10, 0, 2)
        );
        assert_eq!(out.fields, OXYOS_GESTURE_HAS_POSITION);
        assert_eq!(
            (out.x, out.y, out.absolute_x, out.absolute_y),
            (10.0, 20.0, 60.0, 70.0)
        );
        assert_eq!(out.pointer_type, 2);
        assert_eq!(oxyos_gestures_output(gestures, &mut out), 0);
        pointer[0].x = 62.0;
        assert_eq!(touch(OXYOS_GESTURE_UP, &pointer, 1100), 0);
        let mut kinds = Vec::new();
        while oxyos_gestures_output(gestures, &mut out) == 1 {
            kinds.push((out.kind, out.old_state, out.state));
        }
        assert_eq!(
            kinds,
            [
                (OXYOS_GESTURE_TOUCH, 0, 2),
                (OXYOS_GESTURE_TOUCH, 0, 2),
                (OXYOS_GESTURE_INTERCEPT, 0, 0),
                (OXYOS_GESTURE_CANCEL_RESPONDER, 0, 0),
                (OXYOS_GESTURE_STATE_CHANGE, 2, 4),
                (OXYOS_GESTURE_STATE_CHANGE, 4, 5)
            ]
        );
        assert_eq!(oxyos_gestures_next_deadline(gestures), -1);
        // An action on a pointer not given is refused.
        assert_eq!(
            oxyos_gestures_touch(
                gestures,
                OXYOS_GESTURE_MOVE,
                3,
                pointer.as_ptr(),
                1,
                1200,
                1200,
                2,
                hits.as_ptr(),
                0,
                Some(frame),
                std::ptr::null_mut()
            ),
            OXYOS_ERROR
        );
        oxyos_gestures_free(gestures);
    }
}

#[test]
fn the_header_matches_the_pointer_shapes() {
    for (name, value) in [
        ("OXYOS_POINTER_ARROW", OXYOS_POINTER_ARROW),
        ("OXYOS_POINTER_TEXT", OXYOS_POINTER_TEXT),
        ("OXYOS_POINTER_HAND", OXYOS_POINTER_HAND),
        ("OXYOS_POINTER_NOT_ALLOWED", OXYOS_POINTER_NOT_ALLOWED),
        ("OXYOS_POINTER_WAIT", OXYOS_POINTER_WAIT),
        ("OXYOS_POINTER_HELP", OXYOS_POINTER_HELP),
        ("OXYOS_POINTER_CROSSHAIR", OXYOS_POINTER_CROSSHAIR),
        ("OXYOS_POINTER_MOVE", OXYOS_POINTER_MOVE),
        ("OXYOS_POINTER_GRAB", OXYOS_POINTER_GRAB),
        ("OXYOS_POINTER_GRABBING", OXYOS_POINTER_GRABBING),
        (
            "OXYOS_POINTER_RESIZE_HORIZONTAL",
            OXYOS_POINTER_RESIZE_HORIZONTAL,
        ),
        (
            "OXYOS_POINTER_RESIZE_VERTICAL",
            OXYOS_POINTER_RESIZE_VERTICAL,
        ),
        (
            "OXYOS_POINTER_RESIZE_TOP_LEFT",
            OXYOS_POINTER_RESIZE_TOP_LEFT,
        ),
        (
            "OXYOS_POINTER_RESIZE_TOP_RIGHT",
            OXYOS_POINTER_RESIZE_TOP_RIGHT,
        ),
        ("OXYOS_POINTER_ZOOM_IN", OXYOS_POINTER_ZOOM_IN),
        ("OXYOS_POINTER_ZOOM_OUT", OXYOS_POINTER_ZOOM_OUT),
    ] {
        assert_eq!(define(name), i64::from(value), "{name}");
        // Each is the protocol's shape of that number.
        assert!(
            oxyos_protocol::PointerShape::from_raw(value).is_some(),
            "{name}"
        );
    }
    assert!(oxyos_protocol::PointerShape::from_raw(17).is_none());
}

/// `name`'s declaration in the header, as the Rust type of a pointer to it.
fn declaration(name: &str) -> String {
    let start = HEADER
        .find(&format!(" {name}("))
        .unwrap_or_else(|| panic!("{name} is not declared in oxyos_client.h"));
    let line_start = HEADER[..start].rfind('\n').map_or(0, |at| at + 1);
    let returns = HEADER[line_start..start].trim();
    let parameters = &HEADER[start + name.len() + 2..];
    let parameters = &parameters[..parameters.find(");").unwrap()];
    let rust = |c: &str| -> String {
        let c_char = std::any::type_name::<c_char>();
        match c {
            "const char*" => format!("*const {c_char}"),
            "char*" => format!("*mut {c_char}"),
            "size_t" => "usize".into(),
            "double" => "f64".into(),
            "int64_t" => "i64".into(),
            "int64_t*" => "*mut i64".into(),
            "uint32_t*" => "*mut u32".into(),
            "uint64_t*" => "*mut u64".into(),
            "int" => "i32".into(),
            "uint32_t" => "u32".into(),
            "uint64_t" => "u64".into(),
            "bool" => "bool".into(),
            "OxyOSClient*" => format!("*mut {}", std::any::type_name::<OxyOSClient>()),
            "OxyOSPaint*" => format!("*mut {}", std::any::type_name::<crate::paint::OxyOSPaint>()),
            "OxyOSPaintVulkan*" => format!(
                "*mut {}",
                std::any::type_name::<crate::paint::OxyOSPaintVulkan>()
            ),
            "OxyOSPaintPresentTimes*" => format!(
                "*mut {}",
                std::any::type_name::<crate::paint::OxyOSPaintPresentTimes>()
            ),
            "const OxyOSRect*" => format!("*const {}", std::any::type_name::<OxyOSRect>()),
            "size_t*" => "*mut usize".into(),
            "const uint8_t*" => "*const u8".into(),
            "OxyOSFiles*" => format!("*mut {}", std::any::type_name::<crate::files::OxyOSFiles>()),
            "OxyOSFilesReply*" => format!(
                "*mut {}",
                std::any::type_name::<crate::files::OxyOSFilesReply>()
            ),
            "const OxyOSFilesReply*" => format!(
                "*const {}",
                std::any::type_name::<crate::files::OxyOSFilesReply>()
            ),
            "uint8_t" => "u8".into(),
            "OxyOSGpuFormat*" => format!(
                "*mut {}",
                std::any::type_name::<crate::gpu::OxyOSGpuFormat>()
            ),
            "const OxyOSDmabufPlane*" => format!(
                "*const {}",
                std::any::type_name::<crate::gpu::OxyOSDmabufPlane>()
            ),
            "const OxyOSSync*" => {
                format!("*const {}", std::any::type_name::<crate::gpu::OxyOSSync>())
            }
            "const OxyOSSolarAnchor*" => format!(
                "*const {}",
                std::any::type_name::<crate::solar::OxyOSSolarAnchor>()
            ),
            other => panic!("unexpected type {other}"),
        }
    };
    let parameters: Vec<String> = parameters
        .split(',')
        .map(|parameter| {
            let words: Vec<&str> = parameter.split_whitespace().collect();
            rust(&words[..words.len() - 1].join(" "))
        })
        .collect();
    format!(
        "unsafe extern \"C\" fn({}) -> {}",
        parameters.join(", "),
        rust(returns)
    )
}

fn type_of<T>(_: T) -> &'static str {
    std::any::type_name::<T>()
}

#[test]
fn the_header_matches_the_intl_abi() {
    use crate::intl::*;
    type Text = unsafe extern "C" fn(*const c_char, usize, *mut c_char, usize) -> i64;
    type Format = unsafe extern "C" fn(
        *const c_char,
        usize,
        *const c_char,
        usize,
        f64,
        *mut c_char,
        usize,
    ) -> i64;
    type Resolve =
        unsafe extern "C" fn(*const c_char, usize, *const c_char, usize, *mut c_char, usize) -> i64;
    let default: unsafe extern "C" fn(*mut c_char, usize) -> i64 = oxyos_intl_default_locale;
    for (name, rust) in [
        ("oxyos_intl_default_locale", type_of(default)),
        (
            "oxyos_intl_canonicalize",
            type_of(oxyos_intl_canonicalize as Text),
        ),
        ("oxyos_intl_resolve", type_of(oxyos_intl_resolve as Text)),
        (
            "oxyos_intl_format_date",
            type_of(oxyos_intl_format_date as Format),
        ),
        (
            "oxyos_intl_resolve_date",
            type_of(oxyos_intl_resolve_date as Resolve),
        ),
        (
            "oxyos_intl_format_number",
            type_of(oxyos_intl_format_number as Format),
        ),
        (
            "oxyos_intl_resolve_number",
            type_of(oxyos_intl_resolve_number as Resolve),
        ),
    ] {
        assert_eq!(declaration(name), rust, "{name}");
    }
    assert!(HEADER.contains(&format!(
        "Parts are \"type\\x{:X}value\" joined by '\\x{:X}'.",
        UNIT_SEPARATOR as u32, RECORD_SEPARATOR as u32
    )));
}

/// Calls `call` with room for `capacity` bytes, as a C caller does: the length, and the bytes
/// written up to it (or up to the room).
fn answered(capacity: usize, call: impl Fn(*mut c_char, usize) -> i64) -> (i64, Vec<u8>) {
    let mut out = vec![0xff_u8; capacity];
    let length = call(out.as_mut_ptr().cast(), capacity);
    (length, out)
}

/// A call's result, given room enough.
fn intl(call: impl Fn(*mut c_char, usize) -> i64) -> String {
    let (length, out) = answered(1024, call);
    assert!((0..1024).contains(&length), "{length}");
    let length = length as usize;
    assert_eq!(out[length], 0, "NUL-terminated");
    String::from_utf8(out[..length].to_vec()).unwrap()
}

/// The last failure's message.
fn last_error() -> String {
    // SAFETY: oxyos_last_error is NUL-terminated.
    unsafe { CStr::from_ptr(oxyos_last_error()) }
        .to_str()
        .unwrap()
        .into()
}

#[test]
fn intl_through_the_c_abi() {
    use crate::intl::*;
    let text = |text: &str| (text.as_ptr().cast::<c_char>(), text.len());
    let canonical = |tag: &str| {
        let (tag, length) = text(tag);
        // SAFETY: the strings outlive the calls, with their lengths; `out` has the
        // capacity given.
        move |out, capacity| unsafe { oxyos_intl_canonicalize(tag, length, out, capacity) }
    };
    assert_eq!(intl(canonical("EN-us")), "en-US");
    let (requested, length) = text("xx,es-ES-u-hc-h12,fr");
    let resolved =
        // SAFETY: as above.
        intl(|out, capacity| unsafe { oxyos_intl_resolve(requested, length, out, capacity) });
    assert_eq!(resolved, "es-ES-u-hc-h12");
    // SAFETY: as above.
    let default = intl(|out, capacity| unsafe { oxyos_intl_default_locale(out, capacity) });
    assert!(oxyos_intl::canonicalize(&default).is_ok(), "{default}");

    // 2026-10-29T14:05:09Z.
    let (locale, locale_length) = text("es-ES");
    let (options, options_length) = text("day=numeric;month=long;year=numeric;timeZone=UTC");
    // SAFETY: as above.
    let date = intl(|out, capacity| unsafe {
        oxyos_intl_format_date(
            locale,
            locale_length,
            options,
            options_length,
            1_793_282_709_000.0,
            out,
            capacity,
        )
    });
    assert_eq!(
        date,
        "day\x1f29\x1eliteral\x1f de \x1emonth\x1foctubre\x1eliteral\x1f de \x1eyear\x1f2026"
    );
    // SAFETY: as above.
    let resolved = intl(|out, capacity| unsafe {
        oxyos_intl_resolve_date(
            locale,
            locale_length,
            options,
            options_length,
            out,
            capacity,
        )
    });
    assert_eq!(
        resolved,
        "locale=es-ES;calendar=gregory;numberingSystem=latn;timeZone=UTC;year=numeric;\
         month=long;day=numeric"
    );

    let (locale, locale_length) = text("de-DE");
    let (options, options_length) = text("style=currency;currency=EUR");
    // SAFETY: as above.
    let number = intl(|out, capacity| unsafe {
        oxyos_intl_format_number(
            locale,
            locale_length,
            options,
            options_length,
            -1234.5,
            out,
            capacity,
        )
    });
    assert_eq!(
        number,
        "minusSign\x1f-\x1einteger\x1f1\x1egroup\x1f.\x1einteger\x1f234\x1edecimal\x1f,\x1e\
         fraction\x1f50\x1eliteral\x1f\u{a0}\x1ecurrency\x1f€"
    );
    // SAFETY: as above.
    let resolved = intl(|out, capacity| unsafe {
        oxyos_intl_resolve_number(
            locale,
            locale_length,
            options,
            options_length,
            out,
            capacity,
        )
    });
    assert!(
        resolved.starts_with(
            "locale=de-DE;numberingSystem=latn;style=currency;currency=EUR;currencyDisplay=symbol;\
             currencySign=standard;minimumIntegerDigits=1;minimumFractionDigits=2;\
             maximumFractionDigits=2;useGrouping=auto;"
        ),
        "{resolved}"
    );
}

#[test]
fn intl_asks_for_more_room_and_refuses_with_a_range_error() {
    use crate::intl::*;
    let (tag, length) = ("en-us".as_ptr().cast::<c_char>(), 5);
    // Too little room: nothing is written, and the length says how much is needed.
    // SAFETY: the tag outlives the call, with its length; `out` has the capacity given.
    let (needed, out) = answered(3, |out, capacity| unsafe {
        oxyos_intl_canonicalize(tag, length, out, capacity)
    });
    assert_eq!(needed, 5);
    assert_eq!(out, [0xff; 3]);
    // Room for the text but not its NUL: written, not terminated.
    // SAFETY: as above.
    let (written, out) = answered(5, |out, capacity| unsafe {
        oxyos_intl_canonicalize(tag, length, out, capacity)
    });
    assert_eq!((written, out.as_slice()), (5, b"en-US".as_slice()));
    // A size query with no buffer.
    // SAFETY: as above, with no buffer.
    let asked = unsafe { oxyos_intl_canonicalize(tag, length, std::ptr::null_mut(), 0) };
    assert_eq!(asked, 5);

    // An invalid tag, value or unsupported option: -1 and the RangeError's message.
    // SAFETY: as above.
    let (failed, _) = answered(16, |out, capacity| unsafe {
        oxyos_intl_canonicalize("en_US".as_ptr().cast(), 5, out, capacity)
    });
    assert_eq!(failed, -1);
    assert_eq!(
        last_error(),
        "Incorrect locale information provided: \"en_US\""
    );
    let (locale, options) = ("en-US", "timeZoneName=short");
    // SAFETY: the strings outlive the call, with their lengths.
    let failed = unsafe {
        oxyos_intl_format_date(
            locale.as_ptr().cast(),
            locale.len(),
            options.as_ptr().cast(),
            options.len(),
            0.0,
            std::ptr::null_mut(),
            0,
        )
    };
    assert_eq!(failed, -1);
    assert_eq!(
        last_error(),
        "Intl.DateTimeFormat's timeZoneName option is not supported on OxyOS yet"
    );
    let options = "style=currency;currency=EURO";
    // SAFETY: as above.
    let failed = unsafe {
        oxyos_intl_resolve_number(
            locale.as_ptr().cast(),
            locale.len(),
            options.as_ptr().cast(),
            options.len(),
            std::ptr::null_mut(),
            0,
        )
    };
    assert_eq!(failed, -1);
    assert_eq!(last_error(), "Invalid currency code : EURO");
    // A NULL string with a length is refused, not read.
    // SAFETY: NULL with a nonzero length, which the call checks.
    let failed = unsafe { oxyos_intl_resolve(std::ptr::null(), 3, std::ptr::null_mut(), 0) };
    assert_eq!(failed, -1);
    assert_eq!(last_error(), "null locales");
}

#[test]
fn the_header_matches_the_lock_abi() {
    for (name, value) in [
        ("OXYOS_EVENT_LOCK_STATE", i64::from(OXYOS_EVENT_LOCK_STATE)),
        (
            "OXYOS_EVENT_AUTHENTICATION_RESULT",
            OXYOS_EVENT_AUTHENTICATION_RESULT.into(),
        ),
        ("OXYOS_EVENT_CAPS_LOCK", OXYOS_EVENT_CAPS_LOCK.into()),
        (
            "OXYOS_EVENT_LAUNCH_RESULT",
            OXYOS_EVENT_LAUNCH_RESULT.into(),
        ),
        ("OXYOS_AUTH_ACCEPTED", OXYOS_AUTH_ACCEPTED.into()),
        ("OXYOS_AUTH_REJECTED", OXYOS_AUTH_REJECTED.into()),
        ("OXYOS_AUTH_BUSY", OXYOS_AUTH_BUSY.into()),
        ("OXYOS_AUTH_THROTTLED", OXYOS_AUTH_THROTTLED.into()),
    ] {
        assert_eq!(define(name), value, "{name}");
    }
    use oxyos_protocol::AuthOutcome;
    assert_eq!(AuthOutcome::Accepted as u32, OXYOS_AUTH_ACCEPTED);
    assert_eq!(AuthOutcome::Rejected as u32, OXYOS_AUTH_REJECTED);
    assert_eq!(AuthOutcome::Busy as u32, OXYOS_AUTH_BUSY);
    assert_eq!(AuthOutcome::Throttled as u32, OXYOS_AUTH_THROTTLED);
    let caps = convert(&Event::CapsLock {
        surface: 3,
        on: true,
    })
    .unwrap();
    assert_eq!(
        (caps.kind, caps.surface, caps.flag),
        (OXYOS_EVENT_CAPS_LOCK, 3, 1)
    );
    assert_eq!(
        u32::from(
            Event::CapsLock {
                surface: 3,
                on: true
            }
            .opcode()
        ),
        OXYOS_EVENT_CAPS_LOCK
    );
    let state = convert(&Event::LockState { locked: true }).unwrap();
    assert_eq!((state.kind, state.flag), (OXYOS_EVENT_LOCK_STATE, 1));
    let result = convert(&Event::AuthenticationResult {
        surface: 3,
        outcome: AuthOutcome::Throttled,
        wait_s: 60,
    })
    .unwrap();
    assert_eq!(
        (result.kind, result.surface, result.detail, result.code),
        (
            OXYOS_EVENT_AUTHENTICATION_RESULT,
            3,
            OXYOS_AUTH_THROTTLED,
            60
        )
    );
}

#[test]
fn a_password_goes_in_chunks_then_is_submitted() {
    let directory =
        std::env::temp_dir().join(format!("oxyos-client-ffi-lock-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("s");
    let listener = Listener::bind(&socket).unwrap();
    let password = "correct horse battery staple, ñandú";
    let chunks = SecretChunk::split(password).len();
    let compositor = std::thread::spawn(move || {
        let server = listener.accept().unwrap();
        let mut protocol = ServerConnection::new();
        let (hello, _) = server.recv_request().unwrap();
        server
            .send_event(&protocol.receive(&hello).unwrap().unwrap(), None)
            .unwrap();
        (0..chunks + 3)
            .map(|_| server.recv_request().unwrap().0)
            .collect::<Vec<_>>()
    });
    let path = CString::new(socket.to_str().unwrap()).unwrap();
    // SAFETY: the client pointer comes from oxyos_client_connect and is closed once; the strings
    // are as long as said.
    unsafe {
        let client = oxyos_client_connect(path.as_ptr());
        assert!(!client.is_null());
        assert_eq!(oxyos_client_create_lock_surface(client, 4, 1), 0);
        // Too long or not UTF-8, nothing is sent.
        let long = "a".repeat(513);
        assert_eq!(
            oxyos_client_authenticate(client, 4, long.as_ptr().cast(), long.len()),
            OXYOS_ERROR
        );
        assert_eq!(
            oxyos_client_authenticate(client, 4, b"\xff".as_ptr().cast(), 1),
            OXYOS_ERROR
        );
        assert_eq!(
            oxyos_client_authenticate(client, 4, password.as_ptr().cast(), password.len()),
            0
        );
        assert_eq!(oxyos_client_lock_session(client), 0);
        let requests = compositor.join().unwrap();
        assert_eq!(
            requests[0],
            Request::CreateLockSurface {
                surface: 4,
                output: 1
            }
        );
        let given: String = requests[1..=chunks]
            .iter()
            .map(|request| match request {
                Request::AuthenticateSecret { surface: 4, secret } => secret.as_str().to_string(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(given, password);
        assert_eq!(requests[chunks + 1], Request::Authenticate { surface: 4 });
        assert_eq!(requests[chunks + 2], Request::LockSession {});
        oxyos_client_close(client);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_login_screens_password_names_its_account_first() {
    let directory =
        std::env::temp_dir().join(format!("oxyos-client-ffi-login-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("s");
    let listener = Listener::bind(&socket).unwrap();
    let compositor = std::thread::spawn(move || {
        let server = listener.accept().unwrap();
        let mut protocol = ServerConnection::new();
        let (hello, _) = server.recv_request().unwrap();
        server
            .send_event(&protocol.receive(&hello).unwrap().unwrap(), None)
            .unwrap();
        (0..4)
            .map(|_| server.recv_request().unwrap().0)
            .collect::<Vec<_>>()
    });
    let path = CString::new(socket.to_str().unwrap()).unwrap();
    let account = CString::new("oxy-aaaaaaaaaaaaaaaaaaaa").unwrap();
    let too_long = CString::new("a".repeat(32)).unwrap();
    let password = "correct";
    // SAFETY: the client pointer comes from oxyos_client_connect and is closed once; the strings
    // are as long as said, the accounts NUL-terminated.
    unsafe {
        let client = oxyos_client_connect(path.as_ptr());
        assert!(!client.is_null());
        assert_eq!(oxyos_client_create_lock_surface(client, 4, 1), 0);
        // An account no user could have: nothing is sent.
        assert_eq!(
            oxyos_client_authenticate_account(
                client,
                4,
                too_long.as_ptr(),
                password.as_ptr().cast(),
                password.len()
            ),
            OXYOS_ERROR
        );
        assert_eq!(
            oxyos_client_authenticate_account(
                client,
                4,
                account.as_ptr(),
                password.as_ptr().cast(),
                password.len()
            ),
            0
        );
        let requests = compositor.join().unwrap();
        assert_eq!(
            requests[1],
            Request::AuthenticateAccount {
                surface: 4,
                user: PreeditText::new("oxy-aaaaaaaaaaaaaaaaaaaa").unwrap()
            }
        );
        assert_eq!(
            requests[2],
            Request::AuthenticateSecret {
                surface: 4,
                secret: SecretChunk::new(password).unwrap()
            }
        );
        assert_eq!(requests[3], Request::Authenticate { surface: 4 });
        oxyos_client_close(client);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn the_header_matches_the_solar_abi() {
    use crate::solar::*;
    assert_eq!(size_of::<OxyOSSolarAnchor>(), 24);
    assert_eq!(
        [
            offset_of!(OxyOSSolarAnchor, frame),
            offset_of!(OxyOSSolarAnchor, elevation),
            offset_of!(OxyOSSolarAnchor, azimuth),
        ],
        [0, 8, 16]
    );
    type Frame =
        unsafe extern "C" fn(*const OxyOSSolarAnchor, usize, i64, *mut u32, *mut i64) -> c_int;
    assert_eq!(
        declaration("oxyos_solar_frame"),
        type_of(oxyos_solar_frame as Frame)
    );
    // No anchors: an error, nothing written.
    let (mut frame, mut next) = (7u32, 7i64);
    // SAFETY: a null array with a count of 0, and writable outputs.
    let refused = unsafe { oxyos_solar_frame(std::ptr::null(), 0, 0, &mut frame, &mut next) };
    assert_eq!((refused, frame, next), (OXYOS_ERROR, 7, 7));
}

#[test]
fn actions_selections_and_announcements_through_the_c_abi() {
    use oxyos_protocol::{AccessibleActions, Politeness};
    let custom = convert(&Event::AccessibilityAction {
        surface: 2,
        serial: 9,
        index: 4,
        action: AccessibleAction::Custom,
        custom: 1,
        start: 0,
        end: 0,
    })
    .unwrap();
    assert_eq!(
        (custom.detail, custom.code, custom.width),
        (OXYOS_ACTION_CUSTOM_VALUE, 4, 1)
    );
    // An assistive technology's own events never reach an application through this ABI.
    assert!(convert(&Event::AccessibilityAnnouncement {
        client: 1,
        surface: 2,
        index: 0,
        last: true,
        politeness: Politeness::Polite,
        text: oxyos_protocol::PreeditText::new("Hola").unwrap(),
    })
    .is_none());

    let directory = std::env::temp_dir().join(format!(
        "oxyos-client-ffi-a11y-actions-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("s");
    let listener = Listener::bind(&socket).unwrap();
    let compositor = std::thread::spawn(move || {
        let server = listener.accept().unwrap();
        let mut protocol = ServerConnection::new();
        let (hello, _) = server.recv_request().unwrap();
        server
            .send_event(&protocol.receive(&hello).unwrap().unwrap(), None)
            .unwrap();
        let mut requests = Vec::new();
        loop {
            let (request, _) = server.recv_request().unwrap();
            let done = matches!(request, Request::AccessibilityAnnounce { last: true, .. });
            requests.push(request);
            if done {
                break;
            }
        }
        requests
    });
    let path = CString::new(socket.to_str().unwrap()).unwrap();
    let name = CString::new("Volumen").unwrap();
    let value = CString::new("hola").unwrap();
    let labels = CString::new("Reiniciar\nCompartir").unwrap();
    let nodes = [OxyOSAccessibleNode {
        parent: NO_ACCESSIBLE_PARENT,
        role: AccessibleRole::Slider as u8,
        state: AccessibleState::LIVE_POLITE,
        x: 0,
        y: 0,
        width: 100,
        height: 40,
        name: name.as_ptr(),
        value: value.as_ptr(),
        actions: AccessibleActions::INCREMENT | AccessibleActions::SET_SELECTION,
        custom_actions: labels.as_ptr(),
        has_selection: 1,
        selection_start: 1,
        selection_end: 3,
    }];
    let said = "Guardado";
    // SAFETY: the client pointer comes from oxyos_client_connect and is closed once; the nodes,
    // strings and text outlive the calls.
    unsafe {
        let client = oxyos_client_connect(path.as_ptr());
        assert!(!client.is_null());
        // Unknown action bits and politeness are refused before anything is sent.
        let mut bad = nodes;
        bad[0].actions = 1 << 15;
        assert_eq!(
            oxyos_client_publish_accessibility(client, 1, bad.as_ptr(), 1),
            OXYOS_ERROR
        );
        assert_eq!(
            oxyos_client_announce(client, 1, 9, said.as_ptr().cast(), said.len()),
            OXYOS_ERROR
        );
        assert_eq!(
            oxyos_client_announce(client, 1, OXYOS_POLITE_VALUE, b"\xff".as_ptr().cast(), 1),
            OXYOS_ERROR
        );
        assert_eq!(
            oxyos_client_publish_accessibility(client, 1, nodes.as_ptr(), 1),
            0
        );
        assert_eq!(
            oxyos_client_announce(
                client,
                1,
                OXYOS_ASSERTIVE_VALUE,
                said.as_ptr().cast(),
                said.len()
            ),
            0
        );
        let requests = compositor.join().unwrap();
        assert!(requests.contains(&Request::AccessibleNode {
            surface: 1,
            index: 0,
            parent: NO_ACCESSIBLE_PARENT,
            role: AccessibleRole::Slider,
            state: AccessibleState::new(AccessibleState::LIVE_POLITE).unwrap(),
            actions: AccessibleActions::new(
                AccessibleActions::INCREMENT | AccessibleActions::SET_SELECTION
            )
            .unwrap(),
            rect: Rect::new(0, 0, 100, 40),
        }));
        assert!(requests.contains(&Request::AccessibleSelection {
            surface: 1,
            index: 0,
            start: 1,
            end: 3,
        }));
        assert!(requests.iter().any(|r| matches!(
            r,
            Request::AccessibleText { field: oxyos_protocol::AccessibleField::Actions, text, .. }
                if text.as_str() == "Reiniciar\nCompartir"
        )));
        assert!(matches!(
            requests.last(),
            Some(Request::AccessibilityAnnounce {
                surface: 1,
                index: 0,
                last: true,
                politeness: Politeness::Assertive,
                text,
            }) if text.as_str() == "Guardado"
        ));
        oxyos_client_close(client);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

const OXYOS_ACTION_CUSTOM_VALUE: u32 = AccessibleAction::Custom as u32;
const OXYOS_POLITE_VALUE: u8 = oxyos_protocol::Politeness::Polite as u8;
const OXYOS_ASSERTIVE_VALUE: u8 = oxyos_protocol::Politeness::Assertive as u8;

#[test]
fn the_applications_are_listed_as_json_through_the_c_abi() {
    use crate::apps::*;
    let default: unsafe extern "C" fn(*mut c_char, usize) -> i64 = oxyos_applications;
    assert_eq!(declaration("oxyos_applications"), type_of(default));
    // Whatever the machine has: valid JSON-shaped, and asked again with room, the same.
    // SAFETY: a null buffer with no room; then a buffer of the length it said.
    unsafe {
        let length = oxyos_applications(std::ptr::null_mut(), 0);
        assert!(length >= 2);
        let mut buffer = vec![0 as c_char; length as usize + 1];
        assert_eq!(
            oxyos_applications(buffer.as_mut_ptr(), buffer.len()),
            length
        );
        let text = CStr::from_ptr(buffer.as_ptr()).to_str().unwrap();
        assert!(text.starts_with('[') && text.ends_with(']'), "{text}");
    }
}

#[test]
fn the_applications_are_followed_through_the_c_abi() {
    use crate::apps::*;
    for declared in [
        "OxyOSApplicationsWatch* oxyos_applications_watch_new(void);",
        "void oxyos_applications_watch_free(OxyOSApplicationsWatch* watch);",
        "int oxyos_applications_watch_fd(OxyOSApplicationsWatch* watch);",
        "uint8_t oxyos_applications_watch_changed(OxyOSApplicationsWatch* watch);",
    ] {
        assert!(
            HEADER.contains(declared),
            "{declared} is not in oxyos_client.h"
        );
    }
    let new: extern "C" fn() -> *mut OxyOSApplicationsWatch = oxyos_applications_watch_new;
    let fd: unsafe extern "C" fn(*mut OxyOSApplicationsWatch) -> c_int =
        oxyos_applications_watch_fd;
    let changed: unsafe extern "C" fn(*mut OxyOSApplicationsWatch) -> u8 =
        oxyos_applications_watch_changed;
    let free: unsafe extern "C" fn(*mut OxyOSApplicationsWatch) = oxyos_applications_watch_free;
    let watch = new();
    assert!(!watch.is_null());
    // SAFETY: a live watch, freed once; NULL is answered, not dereferenced.
    unsafe {
        assert!(fd(watch) >= 0);
        assert!(changed(watch) <= 1);
        assert_eq!(changed(std::ptr::null_mut()), 0);
        free(watch);
        free(std::ptr::null_mut());
    }
}

#[test]
fn the_shell_asks_for_an_application_through_the_c_abi() {
    assert_eq!(
        u32::from(
            Event::LaunchResult {
                serial: 1,
                launched: true
            }
            .opcode()
        ),
        OXYOS_EVENT_LAUNCH_RESULT
    );
    let result = convert(&Event::LaunchResult {
        serial: 7,
        launched: true,
    })
    .unwrap();
    assert_eq!(
        (result.kind, result.code, result.flag),
        (OXYOS_EVENT_LAUNCH_RESULT, 7, 1)
    );
    let directory =
        std::env::temp_dir().join(format!("oxyos-client-ffi-launch-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("s");
    let listener = Listener::bind(&socket).unwrap();
    let id = "so.oxy.an-application-with-a-rather-long-id";
    let compositor = std::thread::spawn(move || {
        let server = listener.accept().unwrap();
        let mut protocol = ServerConnection::new();
        let (hello, _) = server.recv_request().unwrap();
        server
            .send_event(&protocol.receive(&hello).unwrap().unwrap(), None)
            .unwrap();
        (0..2)
            .map(|_| server.recv_request().unwrap().0)
            .collect::<Vec<_>>()
    });
    let path = CString::new(socket.to_str().unwrap()).unwrap();
    // SAFETY: the client pointer comes from oxyos_client_connect and is closed once; the ids are
    // as long as said.
    unsafe {
        let client = oxyos_client_connect(path.as_ptr());
        assert!(!client.is_null());
        assert_eq!(
            oxyos_client_launch(client, 7, std::ptr::null(), 0),
            OXYOS_ERROR
        );
        let long = "a".repeat(129);
        assert_eq!(
            oxyos_client_launch(client, 7, long.as_ptr().cast(), long.len()),
            OXYOS_ERROR
        );
        assert_eq!(
            oxyos_client_launch(client, 7, id.as_ptr().cast(), id.len()),
            0
        );
        let requests = compositor.join().unwrap();
        let text: String = requests
            .iter()
            .map(|request| match request {
                Request::LaunchApplication {
                    serial: 7, text, ..
                } => text.as_str().to_string(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(text, id);
        assert!(matches!(
            requests.last(),
            Some(Request::LaunchApplication {
                index: 1,
                last: true,
                ..
            })
        ));
        oxyos_client_close(client);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn settings_changes_preferences_and_every_client_is_told_them_through_the_c_abi() {
    for (name, value) in [
        (
            "OXYOS_EVENT_PREFERENCE_RESULT",
            i64::from(OXYOS_EVENT_PREFERENCE_RESULT),
        ),
        ("OXYOS_EVENT_PREFERENCES", OXYOS_EVENT_PREFERENCES.into()),
        ("OXYOS_EVENT_PREFERENCE", OXYOS_EVENT_PREFERENCE.into()),
        ("OXYOS_PREFERENCES_DARK", OXYOS_PREFERENCES_DARK.into()),
        (
            "OXYOS_PREFERENCES_REDUCE_MOTION",
            OXYOS_PREFERENCES_REDUCE_MOTION.into(),
        ),
        (
            "OXYOS_PREFERENCES_REDUCE_TRANSPARENCY",
            OXYOS_PREFERENCES_REDUCE_TRANSPARENCY.into(),
        ),
    ] {
        assert_eq!(define(name), value, "{name}");
    }
    let accent = PreeditText::new("ocean-blue").unwrap();
    for (event, kind) in [
        (
            Event::PreferenceResult {
                serial: 1,
                accepted: true,
            },
            OXYOS_EVENT_PREFERENCE_RESULT,
        ),
        (
            Event::Preferences {
                dark: true,
                reduce_motion: true,
                reduce_transparency: true,
                accent,
            },
            OXYOS_EVENT_PREFERENCES,
        ),
        (
            Event::Preference {
                index: 0,
                last: true,
                text: accent,
            },
            OXYOS_EVENT_PREFERENCE,
        ),
    ] {
        assert_eq!(u32::from(event.opcode()), kind);
    }
    let result = convert(&Event::PreferenceResult {
        serial: 7,
        accepted: false,
    })
    .unwrap();
    assert_eq!(
        (result.kind, result.code, result.flag),
        (OXYOS_EVENT_PREFERENCE_RESULT, 7, 0)
    );
    let preferences = convert(&Event::Preferences {
        dark: true,
        reduce_motion: false,
        reduce_transparency: true,
        accent,
    })
    .unwrap();
    assert_eq!(preferences.kind, OXYOS_EVENT_PREFERENCES);
    assert_eq!(
        preferences.flag,
        OXYOS_PREFERENCES_DARK | OXYOS_PREFERENCES_REDUCE_TRANSPARENCY
    );
    assert_eq!(
        CStr::from_bytes_until_nul(&preferences.text).unwrap(),
        c"ocean-blue"
    );
    let preference = convert(&Event::Preference {
        index: 2,
        last: true,
        text: PreeditText::new("ue").unwrap(),
    })
    .unwrap();
    assert_eq!(
        (preference.kind, preference.detail, preference.flag),
        (OXYOS_EVENT_PREFERENCE, 2, 1)
    );
    assert_eq!(CStr::from_bytes_until_nul(&preference.text).unwrap(), c"ue");

    let directory = std::env::temp_dir().join(format!(
        "oxyos-client-ffi-preference-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("s");
    let listener = Listener::bind(&socket).unwrap();
    let wallpaper = "a-wallpaper-with-a-rather-long-name";
    let compositor = std::thread::spawn(move || {
        let server = listener.accept().unwrap();
        let mut protocol = ServerConnection::new();
        let (hello, _) = server.recv_request().unwrap();
        server
            .send_event(&protocol.receive(&hello).unwrap().unwrap(), None)
            .unwrap();
        // `wallpaper=` and its value: 45 bytes, two chunks.
        (0..2)
            .map(|_| server.recv_request().unwrap().0)
            .collect::<Vec<_>>()
    });
    let path = CString::new(socket.to_str().unwrap()).unwrap();
    // SAFETY: the client pointer comes from oxyos_client_connect and is closed once; the strings
    // are NUL-terminated.
    unsafe {
        let client = oxyos_client_connect(path.as_ptr());
        assert!(!client.is_null());
        assert_eq!(
            oxyos_client_set_preference(client, 7, std::ptr::null(), c"x".as_ptr()),
            OXYOS_ERROR
        );
        assert_eq!(
            oxyos_client_set_preference(client, 7, c"".as_ptr(), c"x".as_ptr()),
            OXYOS_ERROR
        );
        assert_eq!(
            oxyos_client_set_preference(client, 7, c"a=b".as_ptr(), c"x".as_ptr()),
            OXYOS_ERROR
        );
        let long = CString::new("a".repeat(MAX_PREFERENCE_BYTES)).unwrap();
        assert_eq!(
            oxyos_client_set_preference(client, 7, c"wallpaper".as_ptr(), long.as_ptr()),
            OXYOS_ERROR
        );
        let value = CString::new(wallpaper).unwrap();
        assert_eq!(
            oxyos_client_set_preference(client, 7, c"wallpaper".as_ptr(), value.as_ptr()),
            0
        );
        let requests = compositor.join().unwrap();
        let text: String = requests
            .iter()
            .map(|request| match request {
                Request::SetPreference {
                    serial: 7, text, ..
                } => text.as_str().to_string(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(text, format!("wallpaper={wallpaper}"));
        assert!(matches!(
            requests.last(),
            Some(Request::SetPreference {
                index: 1,
                last: true,
                ..
            })
        ));
        oxyos_client_close(client);
    }
    std::fs::remove_dir_all(directory).unwrap();
}
/// The menu bar (ADR 0021).
mod displays;
/// GPU buffers (ADR 0031).
mod gpu;
mod menus;
/// Notifications (ADR 0026).
mod notifications;

#[test]
fn the_header_matches_the_system_shortcuts_abi() {
    for (name, value) in [
        ("OXYOS_EVENT_SYSTEM_SHORTCUT", OXYOS_EVENT_SYSTEM_SHORTCUT),
        ("OXYOS_EVENT_WINDOW_USE", OXYOS_EVENT_WINDOW_USE),
        ("OXYOS_SHORTCUT_SWITCHER_NEXT", OXYOS_SHORTCUT_SWITCHER_NEXT),
        (
            "OXYOS_SHORTCUT_SWITCHER_PREVIOUS",
            OXYOS_SHORTCUT_SWITCHER_PREVIOUS,
        ),
        ("OXYOS_SHORTCUT_SWITCHER_QUIT", OXYOS_SHORTCUT_SWITCHER_QUIT),
        ("OXYOS_SHORTCUT_SWITCHER_HIDE", OXYOS_SHORTCUT_SWITCHER_HIDE),
        ("OXYOS_SHORTCUT_SWITCHER_DONE", OXYOS_SHORTCUT_SWITCHER_DONE),
        (
            "OXYOS_SHORTCUT_SWITCHER_CANCEL",
            OXYOS_SHORTCUT_SWITCHER_CANCEL,
        ),
        ("OXYOS_SHORTCUT_SEARCH", OXYOS_SHORTCUT_SEARCH),
        ("OXYOS_SHORTCUT_MENU_BAR", OXYOS_SHORTCUT_MENU_BAR),
        (
            "OXYOS_MANAGE_HIDE_APPLICATION",
            OXYOS_MANAGE_HIDE_APPLICATION.into(),
        ),
        (
            "OXYOS_MANAGE_QUIT_APPLICATION",
            OXYOS_MANAGE_QUIT_APPLICATION.into(),
        ),
    ] {
        assert_eq!(define(name), i64::from(value), "{name}");
    }
    use oxyos_protocol::SystemShortcut;
    for (shortcut, constant) in [
        (SystemShortcut::SwitcherNext, OXYOS_SHORTCUT_SWITCHER_NEXT),
        (
            SystemShortcut::SwitcherPrevious,
            OXYOS_SHORTCUT_SWITCHER_PREVIOUS,
        ),
        (SystemShortcut::SwitcherQuit, OXYOS_SHORTCUT_SWITCHER_QUIT),
        (SystemShortcut::SwitcherHide, OXYOS_SHORTCUT_SWITCHER_HIDE),
        (SystemShortcut::SwitcherDone, OXYOS_SHORTCUT_SWITCHER_DONE),
        (
            SystemShortcut::SwitcherCancel,
            OXYOS_SHORTCUT_SWITCHER_CANCEL,
        ),
        (SystemShortcut::Search, OXYOS_SHORTCUT_SEARCH),
        (SystemShortcut::MenuBar, OXYOS_SHORTCUT_MENU_BAR),
    ] {
        let event = Event::SystemShortcut {
            shortcut,
            serial: 41,
            output: 2,
        };
        assert_eq!(u32::from(event.opcode()), OXYOS_EVENT_SYSTEM_SHORTCUT);
        let event = convert(&event).unwrap();
        assert_eq!(
            (event.kind, event.serial, event.detail, event.code),
            (OXYOS_EVENT_SYSTEM_SHORTCUT, 41, constant, 2)
        );
    }
    let used = Event::WindowUse {
        window: 7,
        used: 12,
        hidden: true,
    };
    assert_eq!(u32::from(used.opcode()), OXYOS_EVENT_WINDOW_USE);
    let event = convert(&used).unwrap();
    assert_eq!(
        (event.kind, event.code, event.detail, event.flag),
        (OXYOS_EVENT_WINDOW_USE, 7, 12, 1)
    );
    assert!(HEADER.contains(
        "int oxyos_client_focus_shell_surface(OxyOSClient* client, uint32_t surface, uint64_t serial);"
    ));
}

#[test]
fn the_header_matches_the_screenshots_abi() {
    for (name, value) in [
        ("OXYOS_EVENT_CAPTURE_MODE", OXYOS_EVENT_CAPTURE_MODE),
        ("OXYOS_EVENT_CAPTURE_TARGET", OXYOS_EVENT_CAPTURE_TARGET),
        ("OXYOS_EVENT_SCREENSHOT_SAVED", OXYOS_EVENT_SCREENSHOT_SAVED),
        ("OXYOS_CAPTURE_MODE_SELECTION", OXYOS_CAPTURE_MODE_SELECTION),
        ("OXYOS_CAPTURE_MODE_WINDOW", OXYOS_CAPTURE_MODE_WINDOW),
        ("OXYOS_CAPTURE_MODE_TOOLBAR", OXYOS_CAPTURE_MODE_TOOLBAR),
        ("OXYOS_CAPTURE_MODE_CONFIRM", OXYOS_CAPTURE_MODE_CONFIRM),
        ("OXYOS_CAPTURE_MODE_ENDED", OXYOS_CAPTURE_MODE_ENDED),
        ("OXYOS_CAPTURE_SCREENS", OXYOS_CAPTURE_SCREENS.into()),
        ("OXYOS_CAPTURE_AREA", OXYOS_CAPTURE_AREA.into()),
        ("OXYOS_CAPTURE_WINDOW", OXYOS_CAPTURE_WINDOW.into()),
        ("OXYOS_CAPTURE_DESKTOP", OXYOS_CAPTURE_DESKTOP.into()),
        ("OXYOS_CAPTURE_DOCUMENTS", OXYOS_CAPTURE_DOCUMENTS.into()),
    ] {
        assert_eq!(define(name), i64::from(value), "{name}");
    }
    use oxyos_protocol::{CaptureDestination, CaptureKind, CaptureMode};
    for (mode, constant) in [
        (CaptureMode::Selection, OXYOS_CAPTURE_MODE_SELECTION),
        (CaptureMode::Window, OXYOS_CAPTURE_MODE_WINDOW),
        (CaptureMode::Toolbar, OXYOS_CAPTURE_MODE_TOOLBAR),
        (CaptureMode::Confirm, OXYOS_CAPTURE_MODE_CONFIRM),
        (CaptureMode::Ended, OXYOS_CAPTURE_MODE_ENDED),
    ] {
        let event = Event::CaptureMode {
            mode,
            serial: 41,
            output: 2,
        };
        assert_eq!(u32::from(event.opcode()), OXYOS_EVENT_CAPTURE_MODE);
        let event = convert(&event).unwrap();
        assert_eq!(
            (event.kind, event.serial, event.detail, event.code),
            (OXYOS_EVENT_CAPTURE_MODE, 41, constant, 2)
        );
    }
    for (kind, constant) in [
        (CaptureKind::Screens, OXYOS_CAPTURE_SCREENS),
        (CaptureKind::Area, OXYOS_CAPTURE_AREA),
        (CaptureKind::Window, OXYOS_CAPTURE_WINDOW),
    ] {
        assert_eq!(kind as u8, constant);
    }
    assert_eq!(CaptureDestination::Desktop as u8, OXYOS_CAPTURE_DESKTOP);
    assert_eq!(CaptureDestination::Documents as u8, OXYOS_CAPTURE_DOCUMENTS);
    let target = Event::CaptureTarget {
        window: 7,
        rect: Rect::new(-40, 30, 640, 400),
    };
    let event = convert(&target).unwrap();
    assert_eq!(
        (
            event.kind,
            event.code,
            event.x,
            event.y,
            event.width,
            event.height
        ),
        (OXYOS_EVENT_CAPTURE_TARGET, 7, -40, 30, 640, 400)
    );
    let saved = Event::ScreenshotSaved {
        screenshot: 3,
        output: 2,
        width: 2000,
        height: 1250,
        index: 1,
        last: true,
        text: PreeditText::new("a las 10.04.05.png").unwrap(),
    };
    let event = convert(&saved).unwrap();
    assert_eq!(
        (
            event.kind,
            event.code,
            event.x,
            event.width,
            event.height,
            event.detail,
            event.flag
        ),
        (OXYOS_EVENT_SCREENSHOT_SAVED, 3, 2, 2000, 1250, 1, 1)
    );
    let text = CStr::from_bytes_until_nul(&event.text).unwrap();
    assert_eq!(text.to_str().unwrap(), "a las 10.04.05.png");
    for declaration in [
        "int oxyos_client_capture(OxyOSClient* client, uint64_t serial, uint8_t kind, uint32_t output, int32_t x,\n                         int32_t y, uint32_t width, uint32_t height, uint32_t window, uint8_t destination);",
        "int oxyos_client_end_capture(OxyOSClient* client, uint64_t serial);",
        "int oxyos_client_exclude_from_capture(OxyOSClient* client, uint32_t surface, bool excluded);",
        "int oxyos_client_open_screenshot(OxyOSClient* client, uint32_t screenshot, uint64_t serial);",
        "int oxyos_client_set_secure_regions(OxyOSClient* client, uint32_t surface, const OxyOSRect* rects, size_t count);",
    ] {
        assert!(HEADER.contains(declaration), "{declaration}");
    }
}

#[test]
fn the_header_matches_the_window_views_abi() {
    assert_eq!(
        define("OXYOS_EVENT_WINDOW_SIZE"),
        i64::from(OXYOS_EVENT_WINDOW_SIZE)
    );
    let size = Event::WindowSize {
        window: 7,
        size: oxyos_protocol::Size::new(1280, 800),
    };
    assert_eq!(u32::from(size.opcode()), OXYOS_EVENT_WINDOW_SIZE);
    let event = convert(&size).unwrap();
    assert_eq!(
        (event.kind, event.code, event.width, event.height),
        (OXYOS_EVENT_WINDOW_SIZE, 7, 1280, 800)
    );
    // The view as C lays it out: two rectangles of four 4-byte fields after the window, then
    // the radius and the opacity.
    assert!(HEADER.contains(
        "typedef struct OxyOSWindowView {\n  uint32_t window;\n  OxyOSRect rect;\n  OxyOSRect clip;\n  uint32_t radius;\n  uint8_t opacity;\n} OxyOSWindowView;"
    ));
    assert_eq!(
        [
            offset_of!(OxyOSWindowView, window),
            offset_of!(OxyOSWindowView, rect),
            offset_of!(OxyOSWindowView, clip),
            offset_of!(OxyOSWindowView, radius),
            offset_of!(OxyOSWindowView, opacity),
            size_of::<OxyOSWindowView>(),
        ],
        [0, 4, 20, 36, 40, 44]
    );
}

#[test]
fn the_shell_shows_windows_live_through_the_c_abi() {
    let directory =
        std::env::temp_dir().join(format!("oxyos-client-ffi-views-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("s");
    let listener = Listener::bind(&socket).unwrap();
    let compositor = std::thread::spawn(move || {
        let server = listener.accept().unwrap();
        let mut protocol = ServerConnection::new();
        let (hello, _) = server.recv_request().unwrap();
        server
            .send_event(&protocol.receive(&hello).unwrap().unwrap(), None)
            .unwrap();
        (0..4)
            .map(|_| server.recv_request().unwrap().0)
            .collect::<Vec<_>>()
    });
    let path = CString::new(socket.to_str().unwrap()).unwrap();
    let rect = |x, y, width, height| OxyOSRect {
        x,
        y,
        width,
        height,
    };
    let views = [
        OxyOSWindowView {
            window: 7,
            rect: rect(40, 96, 320, 200),
            clip: rect(24, 80, 400, 180),
            radius: 13,
            opacity: 255,
        },
        OxyOSWindowView {
            window: 8,
            rect: rect(400, 96, 160, 100),
            clip: rect(400, 96, 160, 100),
            radius: 0,
            opacity: 64,
        },
    ];
    let too_many = vec![views[0]; oxyos_protocol::MAX_WINDOW_VIEWS + 1];
    // SAFETY: the client pointer comes from oxyos_client_connect and is closed once; the views
    // outlive the calls.
    unsafe {
        let client = oxyos_client_connect(path.as_ptr());
        assert!(!client.is_null());
        assert_eq!(
            oxyos_client_show_windows(client, 3, too_many.as_ptr(), too_many.len()),
            OXYOS_ERROR
        );
        assert_eq!(oxyos_client_show_windows(client, 3, views.as_ptr(), 2), 0);
        assert_eq!(oxyos_client_show_windows(client, 3, std::ptr::null(), 0), 0);
        let requests = compositor.join().unwrap();
        assert_eq!(
            requests,
            [
                Request::ClearWindowViews { surface: 3 },
                Request::PlaceWindowView {
                    surface: 3,
                    window: 7,
                    rect: Rect::new(40, 96, 320, 200),
                    clip: Rect::new(24, 80, 400, 180),
                    radius: 13,
                    opacity: 255,
                },
                Request::PlaceWindowView {
                    surface: 3,
                    window: 8,
                    rect: Rect::new(400, 96, 160, 100),
                    clip: Rect::new(400, 96, 160, 100),
                    radius: 0,
                    opacity: 64,
                },
                Request::ClearWindowViews { surface: 3 },
            ]
        );
        oxyos_client_close(client);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn the_header_matches_the_controls_abi() {
    for (name, value) in [
        ("OXYOS_EVENT_CONTROL_LEVEL", OXYOS_EVENT_CONTROL_LEVEL),
        ("OXYOS_EVENT_POWER", OXYOS_EVENT_POWER),
        ("OXYOS_EVENT_NETWORK", OXYOS_EVENT_NETWORK),
        ("OXYOS_EVENT_BLUETOOTH", OXYOS_EVENT_BLUETOOTH),
        (
            "OXYOS_CONTROL_OUTPUT_VOLUME",
            OXYOS_CONTROL_OUTPUT_VOLUME.into(),
        ),
        (
            "OXYOS_CONTROL_INPUT_VOLUME",
            OXYOS_CONTROL_INPUT_VOLUME.into(),
        ),
        (
            "OXYOS_CONTROL_DISPLAY_BRIGHTNESS",
            OXYOS_CONTROL_DISPLAY_BRIGHTNESS.into(),
        ),
        (
            "OXYOS_CONTROL_KEYBOARD_BRIGHTNESS",
            OXYOS_CONTROL_KEYBOARD_BRIGHTNESS.into(),
        ),
        ("OXYOS_CONTROL_MUTED", OXYOS_CONTROL_MUTED.into()),
        ("OXYOS_CONTROL_AVAILABLE", OXYOS_CONTROL_AVAILABLE.into()),
        ("OXYOS_CONTROL_BEZEL", OXYOS_CONTROL_BEZEL.into()),
        ("OXYOS_BATTERY_ABSENT", OXYOS_BATTERY_ABSENT),
        ("OXYOS_BATTERY_DISCHARGING", OXYOS_BATTERY_DISCHARGING),
        ("OXYOS_BATTERY_CHARGING", OXYOS_BATTERY_CHARGING),
        ("OXYOS_BATTERY_FULL", OXYOS_BATTERY_FULL),
        ("OXYOS_BATTERY_NOT_CHARGING", OXYOS_BATTERY_NOT_CHARGING),
        ("OXYOS_WIFI_UNAVAILABLE", OXYOS_WIFI_UNAVAILABLE),
        ("OXYOS_WIFI_OFF", OXYOS_WIFI_OFF),
        ("OXYOS_WIFI_DISCONNECTED", OXYOS_WIFI_DISCONNECTED),
        ("OXYOS_WIFI_CONNECTING", OXYOS_WIFI_CONNECTING),
        ("OXYOS_WIFI_CONNECTED", OXYOS_WIFI_CONNECTED),
        ("OXYOS_BLUETOOTH_UNAVAILABLE", OXYOS_BLUETOOTH_UNAVAILABLE),
        ("OXYOS_BLUETOOTH_OFF", OXYOS_BLUETOOTH_OFF),
        ("OXYOS_BLUETOOTH_ON", OXYOS_BLUETOOTH_ON),
    ] {
        assert_eq!(define(name), i64::from(value), "{name}");
    }
    use oxyos_protocol::{BatteryState, BluetoothState, Control, WifiState};
    for (control, constant) in [
        (Control::OutputVolume, OXYOS_CONTROL_OUTPUT_VOLUME),
        (Control::InputVolume, OXYOS_CONTROL_INPUT_VOLUME),
        (Control::DisplayBrightness, OXYOS_CONTROL_DISPLAY_BRIGHTNESS),
        (
            Control::KeyboardBrightness,
            OXYOS_CONTROL_KEYBOARD_BRIGHTNESS,
        ),
    ] {
        assert_eq!(control as u8, constant);
    }
    for (state, constant) in [
        (BatteryState::Absent, OXYOS_BATTERY_ABSENT),
        (BatteryState::Discharging, OXYOS_BATTERY_DISCHARGING),
        (BatteryState::Charging, OXYOS_BATTERY_CHARGING),
        (BatteryState::Full, OXYOS_BATTERY_FULL),
        (BatteryState::NotCharging, OXYOS_BATTERY_NOT_CHARGING),
    ] {
        assert_eq!(state as u32, constant);
    }
    for (state, constant) in [
        (WifiState::Unavailable, OXYOS_WIFI_UNAVAILABLE),
        (WifiState::Off, OXYOS_WIFI_OFF),
        (WifiState::Disconnected, OXYOS_WIFI_DISCONNECTED),
        (WifiState::Connecting, OXYOS_WIFI_CONNECTING),
        (WifiState::Connected, OXYOS_WIFI_CONNECTED),
    ] {
        assert_eq!(state as u32, constant);
    }
    for (state, constant) in [
        (BluetoothState::Unavailable, OXYOS_BLUETOOTH_UNAVAILABLE),
        (BluetoothState::Off, OXYOS_BLUETOOTH_OFF),
        (BluetoothState::On, OXYOS_BLUETOOTH_ON),
    ] {
        assert_eq!(state as u32, constant);
    }
    // Each event, field by field.
    let level = convert(&Event::ControlLevel {
        control: Control::DisplayBrightness,
        level_65536: 40_960,
        muted: false,
        available: true,
        bezel: true,
    })
    .unwrap();
    assert_eq!(
        (level.kind, level.detail, level.code, level.flag),
        (
            OXYOS_EVENT_CONTROL_LEVEL,
            u32::from(OXYOS_CONTROL_DISPLAY_BRIGHTNESS),
            40_960,
            OXYOS_CONTROL_AVAILABLE | OXYOS_CONTROL_BEZEL
        )
    );
    let muted = convert(&Event::ControlLevel {
        control: Control::OutputVolume,
        level_65536: 0,
        muted: true,
        available: true,
        bezel: false,
    })
    .unwrap();
    assert_eq!(muted.flag, OXYOS_CONTROL_MUTED | OXYOS_CONTROL_AVAILABLE);
    let power = convert(&Event::Power {
        battery: BatteryState::Charging,
        percent: 78,
        on_ac: true,
        minutes: 181,
    })
    .unwrap();
    assert_eq!(
        (
            power.kind,
            power.detail,
            power.code,
            power.width,
            power.flag
        ),
        (OXYOS_EVENT_POWER, OXYOS_BATTERY_CHARGING, 78, 181, 1)
    );
    let network = convert(&Event::Network {
        wifi: WifiState::Connected,
        signal: 73,
        wired: true,
        ssid: PreeditText::new("LukeSkyRouter").unwrap(),
    })
    .unwrap();
    assert_eq!(
        (network.kind, network.detail, network.code, network.flag),
        (OXYOS_EVENT_NETWORK, OXYOS_WIFI_CONNECTED, 73, 1)
    );
    let ssid = CStr::from_bytes_until_nul(&network.text).unwrap();
    assert_eq!(ssid.to_str().unwrap(), "LukeSkyRouter");
    let bluetooth = convert(&Event::Bluetooth {
        state: BluetoothState::On,
        connected: 2,
    })
    .unwrap();
    assert_eq!(
        (bluetooth.kind, bluetooth.detail, bluetooth.code),
        (OXYOS_EVENT_BLUETOOTH, OXYOS_BLUETOOTH_ON, 2)
    );
    for declaration in [
        "int oxyos_client_set_control_level(OxyOSClient* client, uint8_t control, uint32_t level);",
        "int oxyos_client_set_control_muted(OxyOSClient* client, uint8_t control, bool muted);",
        "int oxyos_client_set_wifi_enabled(OxyOSClient* client, bool enabled);",
        "int oxyos_client_set_appearance(OxyOSClient* client, uint32_t serial, bool dark);",
        "int oxyos_client_refresh_controls(OxyOSClient* client);",
    ] {
        assert!(HEADER.contains(declaration), "{declaration}");
    }
}

#[test]
fn the_control_center_sends_its_requests() {
    let directory =
        std::env::temp_dir().join(format!("oxyos-client-ffi-controls-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("s");
    let listener = Listener::bind(&socket).unwrap();
    let compositor = std::thread::spawn(move || {
        let server = listener.accept().unwrap();
        let mut protocol = ServerConnection::new();
        let (hello, _) = server.recv_request().unwrap();
        server
            .send_event(&protocol.receive(&hello).unwrap().unwrap(), None)
            .unwrap();
        (0..5)
            .map(|_| server.recv_request().unwrap().0)
            .collect::<Vec<_>>()
    });
    let path = CString::new(socket.to_str().unwrap()).unwrap();
    // SAFETY: the client pointer comes from oxyos_client_connect and is closed once.
    unsafe {
        let client = oxyos_client_connect(path.as_ptr());
        assert!(!client.is_null());
        // No such control, and past all of it: refused here, nothing sent.
        assert_eq!(oxyos_client_set_control_level(client, 9, 0), OXYOS_ERROR);
        assert_eq!(
            oxyos_client_set_control_level(client, OXYOS_CONTROL_OUTPUT_VOLUME, 65_537),
            OXYOS_ERROR
        );
        assert_eq!(oxyos_client_set_control_muted(client, 0, true), OXYOS_ERROR);
        assert_eq!(
            oxyos_client_set_control_level(client, OXYOS_CONTROL_DISPLAY_BRIGHTNESS, 32_768),
            0
        );
        assert_eq!(
            oxyos_client_set_control_muted(client, OXYOS_CONTROL_INPUT_VOLUME, true),
            0
        );
        assert_eq!(oxyos_client_set_wifi_enabled(client, false), 0);
        assert_eq!(oxyos_client_set_appearance(client, 4, true), 0);
        assert_eq!(oxyos_client_refresh_controls(client), 0);
        use oxyos_protocol::Control;
        assert_eq!(
            compositor.join().unwrap(),
            [
                Request::SetControlLevel {
                    control: Control::DisplayBrightness,
                    level_65536: 32_768
                },
                Request::SetControlMuted {
                    control: Control::InputVolume,
                    muted: true
                },
                Request::SetWifiEnabled { enabled: false },
                Request::SetAppearance {
                    serial: 4,
                    dark: true
                },
                Request::RefreshControls {},
            ]
        );
        oxyos_client_close(client);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn the_header_matches_the_buffer_abi() {
    let attach: unsafe extern "C" fn(*mut OxyOSClient, c_int, u32, u32, u32, u32, u32) -> i64 =
        oxyos_client_attach_buffer;
    let submit: unsafe extern "C" fn(
        *mut OxyOSClient,
        u32,
        u64,
        u64,
        *const OxyOSRect,
        usize,
    ) -> i64 = oxyos_client_submit_buffer;
    let detach: unsafe extern "C" fn(*mut OxyOSClient, u64) -> c_int = oxyos_client_detach_buffer;
    for (name, rust) in [
        ("oxyos_client_attach_buffer", type_of(attach)),
        ("oxyos_client_submit_buffer", type_of(submit)),
        ("oxyos_client_detach_buffer", type_of(detach)),
    ] {
        assert_eq!(declaration(name), rust, "{name}");
    }
}

/// Sealed shared memory of `bytes`, as a renderer in another process makes it.
fn sealed(bytes: u64) -> rustix::fd::OwnedFd {
    use rustix::fs::{MemfdFlags, SealFlags};
    let fd = rustix::fs::memfd_create("caller", MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING)
        .unwrap();
    rustix::fs::ftruncate(&fd, bytes).unwrap();
    rustix::fs::fcntl_add_seals(&fd, SealFlags::SHRINK | SealFlags::GROW | SealFlags::SEAL)
        .unwrap();
    fd
}

#[test]
fn a_buffer_the_caller_owns_is_presented_without_a_copy() {
    let directory =
        std::env::temp_dir().join(format!("oxyos-client-ffi-buffers-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("s");
    let listener = Listener::bind(&socket).unwrap();
    let compositor = std::thread::spawn(move || {
        let server = listener.accept().unwrap();
        let mut protocol = ServerConnection::new();
        let (hello, _) = server.recv_request().unwrap();
        server
            .send_event(&protocol.receive(&hello).unwrap().unwrap(), None)
            .unwrap();
        let (attach, fd) = server.recv_request().unwrap();
        let layout = attach.buffer_layout().unwrap();
        let shared = SharedBuffer::import(fd.unwrap(), layout).unwrap();
        let damage = server.recv_request().unwrap().0;
        let submit = server.recv_request().unwrap().0;
        let mut row = vec![0; 8];
        shared.read_row(1, &mut row).unwrap();
        server
            .send_event(
                &Event::Released {
                    surface: 3,
                    buffer: 1,
                },
                None,
            )
            .unwrap();
        let destroy = server.recv_request().unwrap().0;
        (attach, damage, submit, row, destroy)
    });

    let path = CString::new(socket.to_str().unwrap()).unwrap();
    let memory = sealed(64);
    rustix::io::pwrite(&memory, &[7, 8, 9, 255, 1, 2, 3, 255], 32).unwrap();
    // SAFETY: the client pointer comes from oxyos_client_connect, is used by this thread only and
    // is closed once at the end; every other pointer is to a live local of the right type, and the
    // descriptors stay open across each call.
    unsafe {
        let client = oxyos_client_connect(path.as_ptr());
        assert!(!client.is_null());
        let unsealed =
            rustix::fs::memfd_create("unsealed", rustix::fs::MemfdFlags::CLOEXEC).unwrap();
        rustix::fs::ftruncate(&unsealed, 64).unwrap();
        assert_eq!(
            oxyos_client_attach_buffer(
                client,
                unsealed.as_raw_fd(),
                OXYOS_FORMAT_ARGB8888,
                2,
                2,
                16,
                16
            ),
            -1,
            "memory that could shrink under the compositor is refused before it is sent"
        );
        assert_eq!(
            oxyos_client_attach_buffer(
                client,
                memory.as_raw_fd(),
                OXYOS_FORMAT_ARGB8888,
                2,
                2,
                16,
                48
            ),
            -1,
            "a layout past the memory's end is refused"
        );
        let buffer = oxyos_client_attach_buffer(
            client,
            memory.as_raw_fd(),
            OXYOS_FORMAT_ARGB8888,
            2,
            2,
            16,
            16,
        );
        assert_eq!(buffer, 1);
        let damage = [OxyOSRect {
            x: 0,
            y: 1,
            width: 9,
            height: 1,
        }];
        assert_eq!(
            oxyos_client_submit_buffer(client, 3, 2, 4, damage.as_ptr(), 1),
            -1,
            "no such buffer"
        );
        assert_eq!(
            oxyos_client_submit_buffer(client, 3, 1, 4, damage.as_ptr(), 1),
            1
        );
        let mut event = OxyOSEvent::default();
        let released = loop {
            match oxyos_client_poll(client, &mut event) {
                OXYOS_POLL_EVENT => break event,
                OXYOS_POLL_EMPTY => std::thread::yield_now(),
                other => panic!("poll returned {other}"),
            }
        };
        assert_eq!(
            (released.kind, released.surface, released.serial),
            (OXYOS_EVENT_RELEASED, 3, 1)
        );
        assert_eq!(oxyos_client_detach_buffer(client, 1), 0);
        assert_eq!(oxyos_client_detach_buffer(client, 1), OXYOS_ERROR);
        let (attach, damage, submit, row, destroy) = compositor.join().unwrap();
        assert_eq!(
            attach,
            Request::AttachBuffer {
                buffer: 1,
                format: Format::Argb8888,
                extent: Size::new(2, 2),
                stride: 16,
                offset: 16
            }
        );
        assert_eq!(
            damage,
            Request::Damage {
                surface: 3,
                rect: Rect::new(0, 1, 2, 1)
            },
            "clipped to the buffer"
        );
        assert_eq!(
            submit,
            Request::Submit {
                surface: 3,
                sequence: 1,
                buffer: 1,
                configure: 4,
                extent: Size::new(2, 2)
            }
        );
        assert_eq!(
            row,
            [7, 8, 9, 255, 1, 2, 3, 255],
            "the caller's pixels, read where it drew them"
        );
        assert_eq!(destroy, Request::DestroyBuffer { buffer: 1 });
        oxyos_client_close(client);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_key_with_its_text_converts_to_a_key_event_carrying_it() {
    let set: unsafe extern "C" fn(*mut OxyOSClient, bool) -> c_int = oxyos_client_set_key_text;
    assert_eq!(declaration("oxyos_client_set_key_text"), type_of(set));
    let event = convert(&Event::KeyText {
        surface: 2,
        serial: 9,
        key: 30,
        pressed: true,
        text: PreeditText::new("é").unwrap(),
        time_ns: 5_000_000_123,
    })
    .unwrap();
    assert_eq!(
        (
            event.kind,
            event.surface,
            event.serial,
            event.code,
            event.flag
        ),
        (OXYOS_EVENT_KEY, 2, 9, 30, 1)
    );
    let text = CStr::from_bytes_until_nul(&event.text).unwrap();
    assert_eq!(text.to_str().unwrap(), "é");
    let plain = convert(&Event::Key {
        surface: 2,
        serial: 10,
        key: 30,
        pressed: false,
        time_ns: 5_000_000_123,
    })
    .unwrap();
    assert_eq!(plain.text[0], 0, "a key without key text has none");
}

#[test]
fn asking_for_key_text_sends_the_request() {
    let directory =
        std::env::temp_dir().join(format!("oxyos-client-ffi-key-text-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("s");
    let listener = Listener::bind(&socket).unwrap();
    let compositor = std::thread::spawn(move || {
        let server = listener.accept().unwrap();
        let mut protocol = ServerConnection::new();
        let (hello, _) = server.recv_request().unwrap();
        server
            .send_event(&protocol.receive(&hello).unwrap().unwrap(), None)
            .unwrap();
        server.recv_request().unwrap().0
    });
    let path = CString::new(socket.to_str().unwrap()).unwrap();
    // SAFETY: the client pointer comes from oxyos_client_connect, is used by this thread only and
    // is closed once.
    unsafe {
        let client = oxyos_client_connect(path.as_ptr());
        assert!(!client.is_null());
        assert_eq!(oxyos_client_set_key_text(client, true), 0);
        oxyos_client_close(client);
    }
    assert_eq!(
        compositor.join().unwrap(),
        Request::SetKeyText { enabled: true }
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn an_applications_directories_are_named_by_its_id_through_the_c_abi() {
    use crate::apps::oxyos_application_directory;
    type Directory = unsafe extern "C" fn(*const c_char, u8, *mut c_char, usize) -> i64;
    assert_eq!(
        declaration("oxyos_application_directory"),
        type_of(oxyos_application_directory as Directory)
    );
    let directory = |id: &str, which: u8| -> Option<String> {
        let id = CString::new(id).unwrap();
        let mut out = [0 as c_char; 512];
        // SAFETY: a NUL-terminated id and a buffer of its length.
        let length =
            unsafe { oxyos_application_directory(id.as_ptr(), which, out.as_mut_ptr(), out.len()) };
        let length = usize::try_from(length).ok()?;
        // SAFETY: the function wrote `length` bytes and a NUL.
        Some(unsafe { CStr::from_ptr(out.as_ptr()) }.to_str().unwrap()[..length].to_string())
    };
    let data = directory("so.oxy.stash", 0).expect("data");
    let cache = directory("so.oxy.stash", 1).expect("cache");
    assert!(data.ends_with("/oxyos/apps/so.oxy.stash"), "{data}");
    assert!(cache.ends_with("/oxyos/apps/so.oxy.stash"), "{cache}");
    assert_ne!(data, cache);
    // An id that is not one never names a directory.
    for id in ["../escape", "", "a/b"] {
        assert_eq!(directory(id, 0), None, "{id}");
    }
    assert_eq!(directory("so.oxy.stash", 2), None);
}

#[test]
fn the_header_matches_the_files_abi() {
    use crate::files::*;
    type New = unsafe extern "C" fn(*const c_char, usize, *const c_char, usize) -> *mut OxyOSFiles;
    type Fd = unsafe extern "C" fn(*mut OxyOSFiles) -> i32;
    type Call = unsafe extern "C" fn(
        *mut OxyOSFiles,
        *const c_char,
        usize,
        *const u8,
        usize,
    ) -> *mut OxyOSFilesReply;
    type Request =
        unsafe extern "C" fn(*mut OxyOSFiles, u64, *const c_char, usize, *const u8, usize) -> i32;
    type Take = unsafe extern "C" fn(*mut OxyOSFiles) -> *mut OxyOSFilesReply;
    type Id = unsafe extern "C" fn(*const OxyOSFilesReply) -> u64;
    type Json = unsafe extern "C" fn(*const OxyOSFilesReply, *mut usize) -> *const c_char;
    type Bytes = unsafe extern "C" fn(*const OxyOSFilesReply, *mut usize) -> *const u8;
    type Open = unsafe extern "C" fn(*mut OxyOSFiles, *const c_char, usize, *mut u64) -> i32;
    for (name, rust) in [
        ("oxyos_files_new", type_of(oxyos_files_new as New)),
        ("oxyos_files_fd", type_of(oxyos_files_fd as Fd)),
        ("oxyos_files_call", type_of(oxyos_files_call as Call)),
        (
            "oxyos_files_request",
            type_of(oxyos_files_request as Request),
        ),
        ("oxyos_files_take", type_of(oxyos_files_take as Take)),
        ("oxyos_files_reply_id", type_of(oxyos_files_reply_id as Id)),
        (
            "oxyos_files_reply_json",
            type_of(oxyos_files_reply_json as Json),
        ),
        (
            "oxyos_files_reply_bytes",
            type_of(oxyos_files_reply_bytes as Bytes),
        ),
        (
            "oxyos_files_open_read",
            type_of(oxyos_files_open_read as Open),
        ),
    ] {
        assert_eq!(declaration(name), rust, "{name}");
    }
    assert!(HEADER.contains("void oxyos_files_free(OxyOSFiles* files);"));
    assert!(HEADER.contains("void oxyos_files_reply_free(OxyOSFilesReply* reply);"));
}

/// A reply's id, JSON and bytes, freed.
///
/// # Safety
/// `reply` is a live reply.
#[allow(unsafe_code)]
unsafe fn files_reply(reply: *mut crate::files::OxyOSFilesReply) -> (u64, String, Vec<u8>) {
    use crate::files::*;
    assert!(!reply.is_null());
    // SAFETY: a live reply, read before it is freed.
    unsafe {
        let mut length = 0;
        let json = oxyos_files_reply_json(reply, &mut length);
        let text = std::slice::from_raw_parts(json.cast::<u8>(), length + 1);
        assert_eq!(text[length], 0, "NUL-terminated");
        let text = String::from_utf8(text[..length].to_vec()).unwrap();
        let mut count = 0;
        let bytes = oxyos_files_reply_bytes(reply, &mut count);
        let bytes = std::slice::from_raw_parts(bytes, count).to_vec();
        let id = oxyos_files_reply_id(reply);
        oxyos_files_reply_free(reply);
        (id, text, bytes)
    }
}

/// The file service through the C ABI, for a home and an application the test makes: the only
/// test that sets the user's directories, which the service reads once, as it starts.
#[test]
fn files_are_read_written_trashed_and_watched_through_the_c_abi() {
    use crate::files::*;
    let home = std::env::temp_dir().join(format!("oxyos-ffi-files-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("Documents")).unwrap();
    std::fs::write(home.join("Documents/notes.txt"), "notes").unwrap();
    let home = std::fs::canonicalize(&home).unwrap();
    std::env::set_var("HOME", &home);
    std::env::set_var("XDG_DATA_HOME", home.join(".local/share"));
    std::env::set_var("XDG_CONFIG_HOME", home.join(".config"));
    std::env::set_var("XDG_CACHE_HOME", home.join(".cache"));
    std::env::set_var("USER", "ffi-test");
    let id = "so.oxy.ffi";
    // SAFETY: a live service, and requests and buffers that outlive each call.
    unsafe {
        assert!(oxyos_files_new("../x".as_ptr().cast(), 4, std::ptr::null(), 0).is_null());
        let files = oxyos_files_new(id.as_ptr().cast(), id.len(), std::ptr::null(), 0);
        assert!(!files.is_null());
        assert!(home.join(".local/share/oxyos/apps/so.oxy.ffi").is_dir());
        assert!(home.join(".cache/oxyos/apps/so.oxy.ffi").is_dir());
        let fd = oxyos_files_fd(files);
        assert!(fd >= 0);
        let call = |json: String, bytes: &[u8]| {
            files_reply(oxyos_files_call(
                files,
                json.as_ptr().cast(),
                json.len(),
                bytes.as_ptr(),
                bytes.len(),
            ))
        };
        let ask = |id: u64, json: String| {
            assert_eq!(
                oxyos_files_request(
                    files,
                    id,
                    json.as_ptr().cast(),
                    json.len(),
                    std::ptr::null(),
                    0
                ),
                0
            );
        };
        // A reply, waited for on the descriptor as the host waits.
        let take = || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let reply = oxyos_files_take(files);
                if !reply.is_null() {
                    return files_reply(reply);
                }
                assert!(std::time::Instant::now() < deadline, "no reply");
                let mut ready = [rustix::event::PollFd::from_borrowed_fd(
                    std::os::fd::BorrowedFd::borrow_raw(fd),
                    rustix::event::PollFlags::IN,
                )];
                let wait = rustix::event::Timespec {
                    tv_sec: 0,
                    tv_nsec: 50_000_000,
                };
                let _ = rustix::event::poll(&mut ready, Some(&wait));
            }
        };
        let documents = format!(
            "file://{}/",
            home.join(".local/share/oxyos/apps/so.oxy.ffi").display()
        );
        let (_, paths, _) = call(r#"{"op":"paths"}"#.into(), &[]);
        assert!(
            paths.contains(&format!(r#""documentDirectory":"{documents}""#)),
            "{paths}"
        );
        // Bytes in and out, at once.
        let data = format!("{documents}data.bin");
        let (_, written, _) = call(
            format!(r#"{{"op":"write","uri":"{data}","encoding":"bytes"}}"#),
            &[0, 1, 2, 255],
        );
        assert_eq!(written, r#"{"ok":true,"value":null}"#);
        let (id0, read, bytes) = call(
            format!(r#"{{"op":"read","uri":"{data}","encoding":"bytes"}}"#),
            &[],
        );
        assert_eq!(
            (id0, read.as_str(), bytes),
            (0, r#"{"ok":true,"value":null}"#, vec![0, 1, 2, 255])
        );
        let (_, refused, _) = call(r#"{"op":"read","uri":"file:///etc/passwd"}"#.into(), &[]);
        assert!(refused.contains(r#""error":"refused""#), "{refused}");
        // In order, through the queue, by id.
        let notes = format!("file://{}/Documents/notes.txt", home.display());
        ask(7, r#"{"op":"watchTrash"}"#.into());
        let (seven, watch, _) = take();
        assert_eq!(seven, 7);
        assert!(watch.contains(r#""value":1"#), "{watch}");
        ask(8, format!(r#"{{"op":"trash","uris":["{notes}"]}}"#));
        let mut heard = [take(), take()];
        heard.sort();
        assert_eq!(heard[0].0, 0);
        assert_eq!(heard[0].1, r#"{"watch":1}"#);
        assert_eq!(heard[1].0, 8);
        assert!(heard[1].1.contains(r#""ok":true"#), "{heard:?}");
        assert!(home.join(".local/share/Trash/files/notes.txt").exists());
        assert!(
            oxyos_files_call(files, [0xffu8].as_ptr().cast(), 1, std::ptr::null(), 0).is_null()
        );
        assert_eq!(
            oxyos_files_request(files, 9, [0xffu8].as_ptr().cast(), 1, std::ptr::null(), 0),
            OXYOS_ERROR
        );
        assert_eq!(
            oxyos_files_request(files, 0, "{}".as_ptr().cast(), 2, std::ptr::null(), 0),
            OXYOS_ERROR
        );
        assert!(oxyos_files_take(files).is_null());
        // A file opened for a request's body: its descriptor and size; nothing outside.
        let mut size = 0;
        let fd = oxyos_files_open_read(files, data.as_ptr().cast(), data.len(), &mut size);
        assert!(fd >= 0 && size == 4);
        let mut read = Vec::new();
        std::io::Read::read_to_end(
            &mut std::fs::File::from(
                <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd),
            ),
            &mut read,
        )
        .unwrap();
        assert_eq!(read, [0, 1, 2, 255]);
        let outside = "file:///etc/passwd";
        assert_eq!(
            oxyos_files_open_read(files, outside.as_ptr().cast(), outside.len(), &mut size),
            OXYOS_ERROR
        );
        oxyos_files_free(files);
    }
    std::fs::remove_dir_all(home).unwrap();
}

/// The painter's structs and calls as the header lays them out (docs/adr/0031, decision 12).
#[test]
fn the_painters_abi_matches_the_header() {
    use crate::paint::{OxyOSPaint, OxyOSPaintPresentTimes, OxyOSPaintVulkan};
    use std::mem::{offset_of, size_of};
    macro_rules! layout {
        ($name:ident, [$($field:ident),*]) => {{
            let found = fields(stringify!($name));
            assert_eq!(names(&found), [$(stringify!($field)),*], stringify!($name));
            let rust = [$(offset_of!($name, $field)),*];
            assert_eq!(c_offsets(&found), (rust.to_vec(), size_of::<$name>()), stringify!($name));
        }};
    }
    layout!(
        OxyOSPaintVulkan,
        [
            get_instance_proc_addr,
            instance,
            physical_device,
            device,
            queue,
            queue_family,
            api_version,
            instance_extensions,
            instance_extension_count,
            device_extensions,
            device_extension_count,
            features
        ]
    );
    layout!(
        OxyOSPaintPresentTimes,
        [
            new_image,
            freed_ms,
            image_ms,
            attach_ms,
            timeline_ms,
            release_ms,
            wait_ms,
            record_ms,
            submit_ms,
            sync_ms,
            send_ms,
            total_ms,
            operations
        ]
    );
    let new: extern "C" fn(u64) -> *mut OxyOSPaint = crate::paint::oxyos_paint_new;
    let vulkan: unsafe extern "C" fn(*mut OxyOSPaint, *mut OxyOSPaintVulkan) -> c_int =
        crate::paint::oxyos_paint_vulkan;
    let present: unsafe extern "C" fn(
        *mut OxyOSPaint,
        *mut OxyOSClient,
        u32,
        u64,
        u64,
        u32,
        u32,
        u32,
        *const OxyOSRect,
        usize,
    ) -> i64 = crate::paint::oxyos_paint_present;
    let last: unsafe extern "C" fn(*mut OxyOSPaint, *mut OxyOSPaintPresentTimes) -> c_int =
        crate::paint::oxyos_paint_last_present;
    assert_eq!(
        declaration("oxyos_paint_new"),
        format!("unsafe {}", type_of(new))
    );
    for (name, rust) in [
        ("oxyos_paint_vulkan", type_of(vulkan)),
        ("oxyos_paint_present", type_of(present)),
        ("oxyos_paint_last_present", type_of(last)),
    ] {
        assert_eq!(declaration(name), rust, "{name}");
    }
}

/// A painter through the C ABI: its Vulkan objects as Skia is given them, and a present without a
/// client refused; skipped without a Vulkan device. Presenting itself is `oxyos-paint`'s test.
#[test]
#[allow(unsafe_code)]
fn a_painter_gives_its_vulkan_objects_through_the_c_abi() {
    use crate::paint::*;
    // SAFETY: every pointer below is live for its call; the painter is freed once.
    unsafe {
        let paint = oxyos_paint_new(0);
        if paint.is_null() {
            eprintln!("no Vulkan device, skipped");
            return;
        }
        assert!(!oxyos_paint_device_name(paint).is_null());
        assert_eq!(oxyos_paint_vulkan(paint, std::ptr::null_mut()), OXYOS_ERROR);
        let mut out = std::mem::MaybeUninit::<OxyOSPaintVulkan>::zeroed();
        assert_eq!(oxyos_paint_vulkan(paint, out.as_mut_ptr()), 0);
        let vulkan = out.assume_init();
        assert!(!vulkan.get_instance_proc_addr.is_null());
        assert!(vulkan.instance != 0 && vulkan.physical_device != 0);
        assert!(vulkan.device != 0 && vulkan.queue != 0);
        assert_eq!(vulkan.api_version, (1 << 22) | (1 << 12), "Vulkan 1.1");
        assert_eq!(vulkan.instance_extension_count, 0);
        let names: Vec<String> =
            std::slice::from_raw_parts(vulkan.device_extensions, vulkan.device_extension_count)
                .iter()
                .map(|&name| CStr::from_ptr(name).to_string_lossy().into_owned())
                .collect();
        // It presents only with DMA-BUF export enabled, and names what it enabled.
        let dmabuf = names
            .iter()
            .any(|name| name == "VK_EXT_external_memory_dma_buf");
        assert!(!oxyos_paint_exports(paint) || dmabuf);
        // VkPhysicalDeviceFeatures2: sType, pNext (none), then robustBufferAccess, disabled.
        let features = vulkan.features.cast::<u8>();
        assert_eq!(
            features.cast::<u32>().read(),
            1_000_059_000,
            "VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2"
        );
        assert!(features.add(8).cast::<*const u8>().read().is_null());
        assert_eq!(features.add(16).cast::<u32>().read(), 0);
        let rect = OxyOSRect {
            x: 0,
            y: 0,
            width: 4,
            height: 4,
        };
        assert_eq!(
            oxyos_paint_present(
                paint,
                std::ptr::null_mut(),
                1,
                1,
                0,
                4,
                4,
                OXYOS_FORMAT_XRGB8888,
                &rect,
                1
            ),
            -1,
            "no client"
        );
        oxyos_paint_free(paint);
    }
}

#[test]
fn the_client_speaks_to_a_version_2_compositor_as_it_writes() {
    let directory =
        std::env::temp_dir().join(format!("oxyos-client-ffi-v2-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("s");
    let listener = Listener::bind(&socket).unwrap();
    let compositor = std::thread::spawn(move || {
        let server = listener.accept().unwrap();
        let (hello, _) = server.recv_request().unwrap();
        assert_eq!(
            hello,
            Request::Hello {
                min_version: 2,
                max_version: oxyos_protocol::VERSION
            }
        );
        server
            .send_event_for(&Event::Welcome { version: 2 }, None, 2)
            .unwrap();
        server
            .send_event_for(
                &Event::Pointer {
                    surface: 1,
                    serial: 4,
                    kind: PointerKind::Press,
                    x: 256,
                    y: 512,
                    button: 272,
                    time_ns: 99,
                    modifiers: 0,
                },
                None,
                2,
            )
            .unwrap();
        server
    });
    let path = CString::new(socket.to_str().unwrap()).unwrap();
    // SAFETY: the client pointer comes from oxyos_client_connect, is used by this thread only and
    // is closed once at the end.
    unsafe {
        let client = oxyos_client_connect(path.as_ptr());
        assert!(!client.is_null(), "a version 2 compositor is spoken to");
        assert_eq!(oxyos_client_protocol_version(client), 2);
        assert_eq!(oxyos_client_protocol_version(std::ptr::null_mut()), 0);
        let mut event = OxyOSEvent::default();
        let pointer = loop {
            match oxyos_client_poll(client, &mut event) {
                OXYOS_POLL_EVENT => break event,
                OXYOS_POLL_EMPTY => std::thread::yield_now(),
                other => panic!("poll: {other}"),
            }
        };
        assert_eq!(
            (pointer.kind, pointer.serial, pointer.code, pointer.time_ns),
            (OXYOS_EVENT_POINTER, 4, 272, 0)
        );
        drop(compositor.join().unwrap());
        oxyos_client_close(client);
    }
}

fn clipboard_entry(mime: &str, data: &[u8]) -> OxyOSClipboardEntry {
    OxyOSClipboardEntry {
        mime: mime.as_ptr().cast(),
        mime_length: mime.len(),
        data: data.as_ptr(),
        length: data.len(),
    }
}

#[test]
fn a_copy_of_any_types_is_offered_whole_and_each_paste_gets_its_own() {
    let directory =
        std::env::temp_dir().join(format!("oxyos-client-ffi-copy-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("s");
    let listener = Listener::bind(&socket).unwrap();
    let compositor = std::thread::spawn(move || {
        let server = listener.accept().unwrap();
        let mut protocol = ServerConnection::new();
        let (hello, _) = server.recv_request().unwrap();
        server
            .send_event(&protocol.receive(&hello).unwrap().unwrap(), None)
            .unwrap();
        let requests: Vec<Request> = (0..5).map(|_| server.recv_request().unwrap().0).collect();
        let mut pastes = Vec::new();
        for mime in ["image/png", "text/html"] {
            let (read, write) = rustix::pipe::pipe().unwrap();
            server
                .send_event(
                    &Event::ClipboardSend {
                        offer: 1,
                        mime: TextChunk::new(mime).unwrap(),
                    },
                    Some(write.as_fd()),
                )
                .unwrap();
            drop(write);
            pastes.push(read);
        }
        server.send_event(&Event::ClipboardEmpty {}, None).unwrap();
        let contents: Vec<Vec<u8>> = pastes
            .into_iter()
            .map(|read| {
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(&mut std::fs::File::from(read), &mut bytes).unwrap();
                bytes
            })
            .collect();
        (requests, contents)
    });

    let path = CString::new(socket.to_str().unwrap()).unwrap();
    let png = [0x89, b'P', b'N', b'G', 0, 255];
    let html = b"<b>hola</b>";
    let text = b"hola";
    // SAFETY: the client pointer comes from oxyos_client_connect, is used by this thread only and
    // is closed once at the end; every entry points to live locals for each call.
    unsafe {
        let client = oxyos_client_connect(path.as_ptr());
        assert!(!client.is_null());
        assert_eq!(
            oxyos_client_copy(client, 9, std::ptr::null(), 0),
            OXYOS_ERROR,
            "a copy of nothing is refused"
        );
        let twice = [
            clipboard_entry("text/html", html),
            clipboard_entry("text/html", text),
        ];
        assert_eq!(
            oxyos_client_copy(client, 9, twice.as_ptr(), 2),
            OXYOS_ERROR,
            "a type named twice is refused"
        );
        let long = "application/x-a-type-past-32-bytes";
        let too_long = [clipboard_entry(long, text)];
        assert_eq!(
            oxyos_client_copy(client, 9, too_long.as_ptr(), 1),
            OXYOS_ERROR
        );
        let many: Vec<String> = (0..17).map(|i| format!("text/x-{i}")).collect();
        let many: Vec<OxyOSClipboardEntry> =
            many.iter().map(|m| clipboard_entry(m, text)).collect();
        assert_eq!(
            oxyos_client_copy(client, 9, many.as_ptr(), many.len()),
            OXYOS_ERROR,
            "more types than an offer takes are refused before anything is sent"
        );
        let entries = [
            clipboard_entry("image/png", &png),
            clipboard_entry("text/html", html),
            clipboard_entry("text/plain;charset=utf-8", text),
        ];
        assert_eq!(oxyos_client_copy(client, 9, entries.as_ptr(), 3), 0);
        let mut event = OxyOSEvent::default();
        loop {
            match oxyos_client_poll(client, &mut event) {
                OXYOS_POLL_EVENT if event.kind == OXYOS_EVENT_CLIPBOARD_EMPTY => break,
                OXYOS_POLL_EVENT | OXYOS_POLL_EMPTY => std::thread::yield_now(),
                other => panic!("poll: {other}"),
            }
        }
        let (requests, contents) = compositor.join().unwrap();
        let mime = |m: &str| TextChunk::new(m).unwrap();
        assert_eq!(
            requests,
            [
                Request::ClipboardOffer {
                    offer: 1,
                    serial: 9
                },
                Request::ClipboardType {
                    offer: 1,
                    mime: mime("image/png")
                },
                Request::ClipboardType {
                    offer: 1,
                    mime: mime("text/html")
                },
                Request::ClipboardType {
                    offer: 1,
                    mime: mime("text/plain;charset=utf-8")
                },
                Request::ClipboardCommit { offer: 1 },
            ]
        );
        assert_eq!(contents, [png.to_vec(), html.to_vec()]);
        oxyos_client_close(client);
    }
}

#[test]
fn the_keyboard_layout_comes_in_chunks_as_the_header_says() {
    assert_eq!(
        define("OXYOS_EVENT_KEYBOARD_LAYOUT"),
        i64::from(OXYOS_EVENT_KEYBOARD_LAYOUT)
    );
    let chunk = convert(&Event::KeyboardLayout {
        index: 0,
        last: true,
        text: oxyos_protocol::PreeditText::new("fr\tazerty\t").unwrap(),
    })
    .unwrap();
    assert_eq!(
        (chunk.kind, chunk.detail, chunk.flag),
        (OXYOS_EVENT_KEYBOARD_LAYOUT, 0, 1)
    );
    assert_eq!(&chunk.text[..11], b"fr\tazerty\t\0");
}

#[test]
fn a_pointer_image_goes_in_a_buffer_of_its_own_set_and_destroyed() {
    let directory =
        std::env::temp_dir().join(format!("oxyos-client-ffi-pointer-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("s");
    let listener = Listener::bind(&socket).unwrap();
    let compositor = std::thread::spawn(move || {
        let server = listener.accept().unwrap();
        let mut protocol = ServerConnection::new();
        let (hello, _) = server.recv_request().unwrap();
        server
            .send_event(&protocol.receive(&hello).unwrap().unwrap(), None)
            .unwrap();
        let (attach, fd) = server.recv_request().unwrap();
        let shared = SharedBuffer::import(fd.unwrap(), attach.buffer_layout().unwrap()).unwrap();
        let mut row = vec![0; 8];
        shared.read_row(1, &mut row).unwrap();
        let set = server.recv_request().unwrap().0;
        let destroy = server.recv_request().unwrap().0;
        (attach, row, set, destroy)
    });
    let path = CString::new(socket.to_str().unwrap()).unwrap();
    let pixels: Vec<u8> = (0..24).collect();
    // SAFETY: the client pointer comes from oxyos_client_connect, is used by this thread only and
    // is closed once at the end; `pixels` holds two rows of 12 bytes.
    unsafe {
        let client = oxyos_client_connect(path.as_ptr());
        assert!(!client.is_null());
        assert_eq!(
            oxyos_client_protocol_version(client),
            oxyos_protocol::VERSION
        );
        assert_eq!(
            oxyos_client_set_pointer_image(client, 1, 2, 2, 4, pixels.as_ptr(), 0, 0, 120),
            OXYOS_ERROR,
            "rows shorter than the image"
        );
        assert_eq!(
            oxyos_client_set_pointer_image(client, 1, 2, 2, 12, pixels.as_ptr(), 2, 0, 120),
            OXYOS_ERROR,
            "a hotspot outside the image"
        );
        assert_eq!(
            oxyos_client_set_pointer_image(client, 1, 300, 2, 1200, pixels.as_ptr(), 0, 0, 120),
            OXYOS_ERROR
        );
        assert_eq!(
            oxyos_client_set_pointer_image(client, 1, 2, 2, 12, pixels.as_ptr(), 1, 1, 180),
            0
        );
        let (attach, row, set, destroy) = compositor.join().unwrap();
        let Request::AttachBuffer {
            buffer,
            format,
            extent,
            stride,
            ..
        } = attach
        else {
            panic!("{attach:?}");
        };
        assert_eq!(
            (format, extent, stride),
            (Format::Argb8888, Size::new(2, 2), 8)
        );
        assert_eq!(
            row,
            [12, 13, 14, 15, 16, 17, 18, 19],
            "the stride's padding is left out"
        );
        assert_eq!(
            set,
            Request::SetPointerImage {
                surface: 1,
                buffer,
                hotspot_x: 1,
                hotspot_y: 1,
                scale_120: 180
            }
        );
        assert_eq!(destroy, Request::DestroyBuffer { buffer });
        oxyos_client_close(client);
    }
}

#[test]
fn outputs_are_told_to_applications_as_the_header_says() {
    for (name, value) in [
        (
            "OXYOS_EVENT_APPLICATION_OUTPUT",
            OXYOS_EVENT_APPLICATION_OUTPUT,
        ),
        (
            "OXYOS_EVENT_APPLICATION_OUTPUT_WORK_AREA",
            OXYOS_EVENT_APPLICATION_OUTPUT_WORK_AREA,
        ),
        (
            "OXYOS_EVENT_APPLICATION_OUTPUT_REMOVED",
            OXYOS_EVENT_APPLICATION_OUTPUT_REMOVED,
        ),
        ("OXYOS_EVENT_SURFACE_OUTPUT", OXYOS_EVENT_SURFACE_OUTPUT),
    ] {
        assert_eq!(define(name), i64::from(value), "{name}");
    }
    let output = convert(&Event::ApplicationOutput {
        output: 2,
        size: Size::new(1280, 720),
        scale_120: 180,
        refresh_mhz: 143_998,
        primary: false,
    })
    .unwrap();
    assert_eq!(
        (
            output.kind,
            output.code,
            output.width,
            output.height,
            output.scale_120,
            output.detail,
            output.flag
        ),
        (
            OXYOS_EVENT_APPLICATION_OUTPUT,
            2,
            1280,
            720,
            180,
            143_998,
            0
        )
    );
    let area = convert(&Event::ApplicationOutputWorkArea {
        output: 2,
        rect: oxyos_protocol::Rect::new(0, 28, 1280, 692),
    })
    .unwrap();
    assert_eq!(
        (
            area.kind,
            area.code,
            area.x,
            area.y,
            area.width,
            area.height
        ),
        (
            OXYOS_EVENT_APPLICATION_OUTPUT_WORK_AREA,
            2,
            0,
            28,
            1280,
            692
        )
    );
    let on = convert(&Event::SurfaceOutput {
        surface: 3,
        output: 2,
    })
    .unwrap();
    assert_eq!(
        (on.kind, on.surface, on.code),
        (OXYOS_EVENT_SURFACE_OUTPUT, 3, 2)
    );
    let gone = convert(&Event::ApplicationOutputRemoved { output: 2 }).unwrap();
    assert_eq!(
        (gone.kind, gone.code),
        (OXYOS_EVENT_APPLICATION_OUTPUT_REMOVED, 2)
    );
}

#[test]
fn files_are_asked_for_answered_and_chosen_through_the_c_abi() {
    assert_eq!(
        define("OXYOS_EVENT_FILES_CHOSEN"),
        i64::from(OXYOS_EVENT_FILES_CHOSEN)
    );
    for declaration in [
        "int oxyos_client_choose_files(OxyOSClient* client, uint32_t serial, uint32_t surface, uint64_t input,\n                              const char* text);",
        "int oxyos_client_cancel_choose_files(OxyOSClient* client, uint32_t serial);",
        "int oxyos_client_answer_file_chooser(OxyOSClient* client, uint64_t input, const char* text);",
    ] {
        assert!(HEADER.contains(declaration), "{declaration}");
    }
    let chosen = Event::FilesChosen {
        serial: 7,
        index: 2,
        last: true,
        text: PreeditText::new(r#"/a%20b.png"]}"#).unwrap(),
    };
    assert_eq!(u32::from(chosen.opcode()), OXYOS_EVENT_FILES_CHOSEN);
    let event = convert(&chosen).unwrap();
    assert_eq!(
        (event.kind, event.code, event.detail, event.flag),
        (OXYOS_EVENT_FILES_CHOSEN, 7, 2, 1)
    );
    let text = CStr::from_bytes_until_nul(&event.text).unwrap();
    assert_eq!(text.to_str().unwrap(), r#"/a%20b.png"]}"#);

    // A compositor of version 7 is sent the request in chunks, the withdrawal and an answer.
    let directory =
        std::env::temp_dir().join(format!("oxyos-client-ffi-choose-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("s");
    let listener = Listener::bind(&socket).unwrap();
    let request = r#"{"open":{"multiple":true,"types":["image/png",".png"]}}"#;
    let compositor = std::thread::spawn(move || {
        let server = listener.accept().unwrap();
        let mut protocol = ServerConnection::new();
        let (hello, _) = server.recv_request().unwrap();
        server
            .send_event(&protocol.receive(&hello).unwrap().unwrap(), None)
            .unwrap();
        let mut received = Vec::new();
        loop {
            let (request, _) = server.recv_request().unwrap();
            let done = matches!(request, Request::AnswerFileChooser { last: true, .. });
            received.push(request);
            if done {
                break received;
            }
        }
    });
    let path = CString::new(socket.to_str().unwrap()).unwrap();
    let text = CString::new(request).unwrap();
    let answer = CString::new(r#"{"cancelled":{}}"#).unwrap();
    let empty = CString::new("").unwrap();
    // SAFETY: the client pointer comes from oxyos_client_connect and is closed once; the strings
    // outlive the calls.
    let received = unsafe {
        let client = oxyos_client_connect(path.as_ptr());
        assert!(!client.is_null());
        assert_eq!(
            oxyos_client_protocol_version(client),
            oxyos_protocol::VERSION
        );
        assert_eq!(
            oxyos_client_choose_files(client, 7, 1, 42, text.as_ptr()),
            0
        );
        assert_eq!(
            oxyos_client_choose_files(client, 8, 1, 42, std::ptr::null()),
            OXYOS_ERROR
        );
        assert_eq!(
            oxyos_client_choose_files(client, 8, 1, 42, empty.as_ptr()),
            OXYOS_ERROR
        );
        assert_eq!(oxyos_client_cancel_choose_files(client, 7), 0);
        assert_eq!(
            oxyos_client_answer_file_chooser(client, 0, answer.as_ptr()),
            0
        );
        let received = compositor.join().unwrap();
        oxyos_client_close(client);
        received
    };
    let asked: Vec<(u32, u32, u64, u16, bool, String)> = received
        .iter()
        .filter_map(|request| match request {
            Request::ChooseFiles {
                serial,
                surface,
                input,
                index,
                last,
                text,
            } => Some((
                *serial,
                *surface,
                *input,
                *index,
                *last,
                text.as_str().to_string(),
            )),
            _ => None,
        })
        .collect();
    assert!(
        asked.len() > 1,
        "a request longer than a chunk goes in several"
    );
    for (number, (serial, surface, input, index, last, _)) in asked.iter().enumerate() {
        assert_eq!((*serial, *surface, *input), (7, 1, 42));
        assert_eq!(usize::from(*index), number);
        assert_eq!(*last, number + 1 == asked.len());
    }
    let whole: String = asked.iter().map(|chunk| chunk.5.as_str()).collect();
    assert_eq!(whole, request);
    assert!(received.contains(&Request::CancelChooseFiles { serial: 7 }));
    assert_eq!(
        received.last(),
        Some(&Request::AnswerFileChooser {
            input: 0,
            index: 0,
            last: true,
            text: PreeditText::new(r#"{"cancelled":{}}"#).unwrap(),
        })
    );
    std::fs::remove_dir_all(directory).unwrap();

    // A compositor of version 6 is never sent a request it would close the connection for.
    let directory =
        std::env::temp_dir().join(format!("oxyos-client-ffi-choose-6-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("s");
    let listener = Listener::bind(&socket).unwrap();
    let compositor = std::thread::spawn(move || {
        let server = listener.accept().unwrap();
        server.recv_request().unwrap();
        server
            .send_event_for(&Event::Welcome { version: 6 }, None, 6)
            .unwrap();
        server.recv_request().map(|(request, _)| request)
    });
    let path = CString::new(socket.to_str().unwrap()).unwrap();
    // SAFETY: as above.
    unsafe {
        let client = oxyos_client_connect(path.as_ptr());
        assert!(!client.is_null());
        assert_eq!(oxyos_client_protocol_version(client), 6);
        assert_eq!(
            oxyos_client_choose_files(client, 7, 1, 42, text.as_ptr()),
            OXYOS_ERROR
        );
        assert_eq!(oxyos_client_cancel_choose_files(client, 7), OXYOS_ERROR);
        assert_eq!(
            oxyos_client_answer_file_chooser(client, 0, answer.as_ptr()),
            OXYOS_ERROR
        );
        oxyos_client_close(client);
    }
    assert!(
        compositor.join().unwrap().is_err(),
        "nothing was sent before the connection closed"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn the_docks_requests_and_events_go_through_the_c_abi() {
    for (name, value) in [
        (
            "OXYOS_EVENT_APPLICATION_LIST",
            i64::from(OXYOS_EVENT_APPLICATION_LIST),
        ),
        (
            "OXYOS_EVENT_TRASH_ITEMS",
            i64::from(OXYOS_EVENT_TRASH_ITEMS),
        ),
        (
            "OXYOS_APPLICATION_LIST_DOCK",
            i64::from(OXYOS_APPLICATION_LIST_DOCK),
        ),
        (
            "OXYOS_APPLICATION_LIST_OPEN_AT_LOGIN",
            i64::from(OXYOS_APPLICATION_LIST_OPEN_AT_LOGIN),
        ),
        ("OXYOS_LIST_LAST", i64::from(OXYOS_LIST_LAST)),
        ("OXYOS_LIST_CHOSEN", i64::from(OXYOS_LIST_CHOSEN)),
        (
            "OXYOS_WINDOW_RESPONDING",
            i64::from(OXYOS_WINDOW_RESPONDING),
        ),
        ("OXYOS_MODIFIER_SHIFT", i64::from(OXYOS_MODIFIER_SHIFT)),
        ("OXYOS_MODIFIER_CONTROL", i64::from(OXYOS_MODIFIER_CONTROL)),
        ("OXYOS_MODIFIER_ALT", i64::from(OXYOS_MODIFIER_ALT)),
        ("OXYOS_MODIFIER_SUPER", i64::from(OXYOS_MODIFIER_SUPER)),
        (
            "OXYOS_MANAGE_FORCE_QUIT_APPLICATION",
            i64::from(OXYOS_MANAGE_FORCE_QUIT_APPLICATION),
        ),
    ] {
        assert_eq!(define(name), value, "{name}");
    }
    for declaration in [
        "int oxyos_client_set_application_list(OxyOSClient* client, uint32_t serial, uint32_t list, const char* ids,\n                                      size_t length);",
        "int oxyos_client_open_url(OxyOSClient* client, uint32_t serial, uint64_t input, const char* url,\n                          size_t length);",
        "int oxyos_client_empty_trash(OxyOSClient* client, uint64_t input);",
    ] {
        assert!(HEADER.contains(declaration), "{declaration}");
    }
    // The session's list and the Trash, to the shell; a Ping never reaches a C client.
    let list = convert(&Event::ApplicationList {
        list: oxyos_protocol::ApplicationListKind::Dock,
        chosen: true,
        index: 1,
        last: true,
        text: PreeditText::new("so.oxy.inbox").unwrap(),
    })
    .unwrap();
    assert_eq!(
        (list.kind, list.code, list.detail, list.flag),
        (
            OXYOS_EVENT_APPLICATION_LIST,
            OXYOS_APPLICATION_LIST_DOCK,
            1,
            OXYOS_LIST_LAST | OXYOS_LIST_CHOSEN
        )
    );
    assert_eq!(
        CStr::from_bytes_until_nul(&list.text)
            .unwrap()
            .to_str()
            .unwrap(),
        "so.oxy.inbox"
    );
    let trash = convert(&Event::TrashItems { items: 4 }).unwrap();
    assert_eq!((trash.kind, trash.code), (OXYOS_EVENT_TRASH_ITEMS, 4));
    assert!(convert(&Event::Ping { serial: 1 }).is_none());

    // A compositor of version 8 is sent the list, the URL, the force quit and the Trash's emptying,
    // and its Ping is answered by the library as the client reads.
    let directory =
        std::env::temp_dir().join(format!("oxyos-client-ffi-dock-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("s");
    let listener = Listener::bind(&socket).unwrap();
    let compositor = std::thread::spawn(move || {
        let server = listener.accept().unwrap();
        let mut protocol = ServerConnection::new();
        let (hello, _) = server.recv_request().unwrap();
        server
            .send_event(&protocol.receive(&hello).unwrap().unwrap(), None)
            .unwrap();
        server.send_event(&Event::Ping { serial: 5 }, None).unwrap();
        server
            .send_event(&Event::TrashItems { items: 2 }, None)
            .unwrap();
        let mut received = Vec::new();
        loop {
            let (request, _) = server.recv_request().unwrap();
            let done = matches!(request, Request::EmptyTrash { .. });
            received.push(request);
            if done {
                break received;
            }
        }
    });
    let path = CString::new(socket.to_str().unwrap()).unwrap();
    let ids = "so.oxy.stash\nso.oxy.allo";
    let url = "stash://trash";
    // SAFETY: the client pointer comes from oxyos_client_connect and is closed once; the buffers
    // outlive the calls.
    let received = unsafe {
        let client = oxyos_client_connect(path.as_ptr());
        assert!(!client.is_null());
        let mut event = OxyOSEvent::default();
        // The Ping is answered and skipped: the first event the client is given is the Trash's.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while oxyos_client_poll(client, &mut event) != OXYOS_POLL_EVENT {
            assert!(std::time::Instant::now() < deadline, "no event");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!((event.kind, event.code), (OXYOS_EVENT_TRASH_ITEMS, 2));
        assert_eq!(
            oxyos_client_set_application_list(
                client,
                3,
                OXYOS_APPLICATION_LIST_DOCK,
                ids.as_ptr().cast(),
                ids.len()
            ),
            0
        );
        assert_eq!(
            oxyos_client_set_application_list(client, 4, 9, ids.as_ptr().cast(), ids.len()),
            OXYOS_ERROR
        );
        assert_eq!(
            oxyos_client_open_url(client, 6, 42, url.as_ptr().cast(), url.len()),
            0
        );
        assert_eq!(
            oxyos_client_manage(client, OXYOS_MANAGE_FORCE_QUIT_APPLICATION, 7, 0),
            0
        );
        assert_eq!(oxyos_client_empty_trash(client, 43), 0);
        let received = compositor.join().unwrap();
        oxyos_client_close(client);
        received
    };
    assert_eq!(
        received,
        [
            Request::Pong { serial: 5 },
            Request::SetApplicationList {
                serial: 3,
                list: oxyos_protocol::ApplicationListKind::Dock,
                index: 0,
                last: true,
                text: PreeditText::new("so.oxy.stash\nso.oxy.allo").unwrap(),
            },
            Request::OpenUrl {
                serial: 6,
                input: 42,
                index: 0,
                last: true,
                text: PreeditText::new("stash://trash").unwrap(),
            },
            Request::ForceQuitApplication { window: 7 },
            Request::EmptyTrash { input: 43 },
        ]
    );
    std::fs::remove_dir_all(directory).unwrap();
}
