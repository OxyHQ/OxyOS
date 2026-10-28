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
                "int32_t" | "uint32_t" => 4,
                "uint64_t" | "int64_t" | "size_t" => 8,
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
