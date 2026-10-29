#![allow(unsafe_code)]

use super::*;
use oxyos_ipc::{Listener, SharedBuffer};
use oxyos_protocol::{
    AccessibleAction, DismissReason, PointerKind, ServerConnection, NO_ACCESSIBLE_PARENT,
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
    let body = &body[body.find('{').unwrap() + 1..body.find('}').unwrap()];
    body.split(';')
        .filter_map(|field| {
            let mut words = field.split_whitespace();
            let mut kind = words.next()?;
            if kind == "const" {
                kind = words.next()?;
            }
            let declarator = words.next()?;
            let size = match kind {
                "uint8_t" | "char" => 1,
                "uint16_t" => 2,
                "int32_t" | "uint32_t" | "float" => 4,
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
        ["parent", "role", "state", "x", "y", "width", "height", "name", "value"]
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
        offset_of!(OxyOSEvent, flag),
        offset_of!(OxyOSEvent, text),
    ];
    assert_eq!(event.last().map(|f| f.count), Some(TEXT_CHUNK_BYTES + 1));
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
            "flag",
            "text"
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
    })
    .unwrap();
    assert_eq!(
        (scroll.kind, scroll.x, scroll.y),
        (OXYOS_EVENT_SCROLL, 0, 512)
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
                    action: AccessibleAction::Activate,
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
                action.detail
            ),
            (
                OXYOS_EVENT_ACCESSIBILITY_ACTION,
                1,
                77,
                1,
                AccessibleAction::Activate as u32
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
        ("OXYOS_NET_RESPONSE", OXYOS_NET_RESPONSE.into()),
        ("OXYOS_NET_CHUNK", OXYOS_NET_CHUNK.into()),
        ("OXYOS_NET_DATA", OXYOS_NET_DATA.into()),
        ("OXYOS_NET_COMPLETE", OXYOS_NET_COMPLETE.into()),
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
            "incremental"
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
    let (method, url, body) = ("PUT", format!("http://{address}/thing"), "hola");
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
        body: body.as_ptr(),
        body_length: body.len(),
        timeout_ms: 5000,
        has_body: 1,
        with_credentials: 1,
        incremental: 0,
    };
    // SAFETY: a live network, and a request whose pointers outlive the call.
    assert_eq!(unsafe { oxyos_net_send(network, &request) }, 0);

    let mut got = Vec::new();
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
    assert_eq!(define("OXYOS_IMAGE_ANSWER"), OXYOS_IMAGE_ANSWER.into());
    assert_eq!(define("OXYOS_IMAGE_EMPTY"), OXYOS_IMAGE_EMPTY.into());
    assert_eq!(
        define("OXYOS_IMAGE_FIT_WITHIN"),
        OXYOS_IMAGE_FIT_WITHIN.into()
    );
    assert_eq!(
        define("OXYOS_IMAGE_FIT_COVERING"),
        OXYOS_IMAGE_FIT_COVERING.into()
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
fn the_header_matches_the_vector_abi() {
    use crate::vector::*;
    for (name, value) in [
        ("OXYOS_SHAPE_PATH", OXYOS_SHAPE_PATH),
        ("OXYOS_SHAPE_RECT", OXYOS_SHAPE_RECT),
        ("OXYOS_SHAPE_ELLIPSE", OXYOS_SHAPE_ELLIPSE),
        ("OXYOS_SHAPE_LINE", OXYOS_SHAPE_LINE),
        ("OXYOS_CAP_BUTT", OXYOS_CAP_BUTT),
        ("OXYOS_CAP_ROUND", OXYOS_CAP_ROUND),
        ("OXYOS_CAP_SQUARE", OXYOS_CAP_SQUARE),
        ("OXYOS_JOIN_MITER", OXYOS_JOIN_MITER),
        ("OXYOS_JOIN_ROUND", OXYOS_JOIN_ROUND),
        ("OXYOS_JOIN_BEVEL", OXYOS_JOIN_BEVEL),
    ] {
        assert_eq!(define(name), i64::from(value), "{name}");
    }
    let shape = fields("OxyOSVectorShape");
    assert_eq!(
        names(&shape),
        [
            "kind",
            "data",
            "data_length",
            "geometry",
            "transform",
            "fill",
            "even_odd",
            "fill_color",
            "stroke",
            "cap",
            "join",
            "stroke_color",
            "stroke_width",
            "miter_limit",
            "dash",
            "dash_count",
            "dash_offset"
        ]
    );
    let rust = [
        offset_of!(OxyOSVectorShape, kind),
        offset_of!(OxyOSVectorShape, data),
        offset_of!(OxyOSVectorShape, data_length),
        offset_of!(OxyOSVectorShape, geometry),
        offset_of!(OxyOSVectorShape, transform),
        offset_of!(OxyOSVectorShape, fill),
        offset_of!(OxyOSVectorShape, even_odd),
        offset_of!(OxyOSVectorShape, fill_color),
        offset_of!(OxyOSVectorShape, stroke),
        offset_of!(OxyOSVectorShape, cap),
        offset_of!(OxyOSVectorShape, join),
        offset_of!(OxyOSVectorShape, stroke_color),
        offset_of!(OxyOSVectorShape, stroke_width),
        offset_of!(OxyOSVectorShape, miter_limit),
        offset_of!(OxyOSVectorShape, dash),
        offset_of!(OxyOSVectorShape, dash_count),
        offset_of!(OxyOSVectorShape, dash_offset),
    ];
    assert_eq!(
        c_offsets(&shape),
        (rust.to_vec(), size_of::<OxyOSVectorShape>())
    );
}

#[test]
fn shapes_are_drawn_through_the_c_abi() {
    use crate::vector::*;
    let data = "M0 0H4V4H0Z";
    let shape = |kind, data: &str| OxyOSVectorShape {
        kind,
        data: data.as_ptr().cast(),
        data_length: data.len(),
        geometry: [4.0, 4.0, 4.0, 4.0, 0.0, 0.0],
        transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        fill: 1,
        even_odd: 0,
        fill_color: 0xff00_ff00,
        stroke: 0,
        cap: OXYOS_CAP_BUTT,
        join: OXYOS_JOIN_MITER,
        stroke_color: 0,
        stroke_width: 0.0,
        miter_limit: 4.0,
        dash: std::ptr::null(),
        dash_count: 0,
        dash_offset: 0.0,
    };
    let shapes = [
        shape(OXYOS_SHAPE_PATH, data),
        shape(OXYOS_SHAPE_RECT, ""),
        shape(OXYOS_SHAPE_PATH, "M0 0 L"),
    ];
    let mut pixels = vec![0u8; 8 * 8 * 4];
    // SAFETY: the shapes, their data and the pixels outlive the call, with the counts given.
    let skipped = unsafe {
        oxyos_vector_render(
            shapes.as_ptr(),
            shapes.len(),
            8,
            8,
            pixels.as_mut_ptr(),
            pixels.len(),
        )
    };
    let pixel = |x: usize, y: usize| &pixels[(y * 8 + x) * 4..(y * 8 + x) * 4 + 4];
    assert_eq!(skipped, 1);
    // Green, opaque: R, G, B, A.
    assert_eq!(pixel(0, 0), [0, 255, 0, 255]);
    assert_eq!(pixel(5, 5), [0, 255, 0, 255]);
    assert_eq!(pixel(5, 0), [0, 0, 0, 0]);
    // Too few bytes for the picture, or an unknown kind: refused.
    // SAFETY: as above, with a length smaller than the picture.
    let refused =
        unsafe { oxyos_vector_render(shapes.as_ptr(), 1, 8, 8, pixels.as_mut_ptr(), 255) };
    assert_eq!(refused, OXYOS_ERROR);
    let unknown = [shape(9, data)];
    // SAFETY: as above.
    let refused =
        unsafe { oxyos_vector_render(unknown.as_ptr(), 1, 8, 8, pixels.as_mut_ptr(), 256) };
    assert_eq!(refused, OXYOS_ERROR);
}

#[test]
fn the_header_matches_the_shell_surface_abi() {
    for (name, value) in [
        ("OXYOS_SHELL_BACKGROUND", ShellKind::Background as i64),
        ("OXYOS_SHELL_PANEL", ShellKind::Panel as i64),
        ("OXYOS_SHELL_OVERLAY", ShellKind::Overlay as i64),
        ("OXYOS_SHELL_NOTIFICATION", ShellKind::Notification as i64),
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
        (
            "OXYOS_EVENT_CLOSE_REQUESTED",
            OXYOS_EVENT_CLOSE_REQUESTED.into(),
        ),
        ("OXYOS_WINDOW_MINIMIZED", OXYOS_WINDOW_MINIMIZED.into()),
        ("OXYOS_WINDOW_FOCUSED", OXYOS_WINDOW_FOCUSED.into()),
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
    };
    assert_eq!(u32::from(state.opcode()), OXYOS_EVENT_WINDOW_STATE);
    assert_eq!(
        u32::from(Event::CloseRequested { surface: 1 }.opcode()),
        OXYOS_EVENT_CLOSE_REQUESTED
    );
}

#[test]
fn window_events_convert_field_by_field() {
    let state = convert(&Event::WindowState {
        window: 7,
        workspace: 2,
        minimized: true,
        focused: true,
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
            "duration"
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
        let pinch = "PinchGestureHandler";
        assert_eq!(
            oxyos_gestures_create(
                gestures,
                pinch.as_ptr().cast(),
                pinch.len(),
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
