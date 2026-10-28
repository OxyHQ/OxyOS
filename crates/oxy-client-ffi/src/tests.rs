#![allow(unsafe_code)]

use super::*;
use oxy_ipc::{Listener, SharedBuffer};
use oxy_protocol::{DismissReason, PointerKind, ServerConnection};
use std::mem::{offset_of, size_of};

const HEADER: &str = include_str!("../include/oxy_client.h");

/// `#define NAME value` from the header.
fn define(name: &str) -> i64 {
    let line = HEADER
        .lines()
        .find(|line| line.split_whitespace().nth(1) == Some(name))
        .unwrap_or_else(|| panic!("{name} is not defined in oxy_client.h"));
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
            let (kind, declarator) = (words.next()?, words.next()?);
            let size = match kind {
                "uint8_t" | "char" => 1,
                "int32_t" | "uint32_t" => 4,
                "uint64_t" => 8,
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
        ("OXY_EVENT_CONFIGURE", OXY_EVENT_CONFIGURE as i64),
        ("OXY_EVENT_POPUP_DONE", OXY_EVENT_POPUP_DONE.into()),
        ("OXY_EVENT_POINTER", OXY_EVENT_POINTER.into()),
        ("OXY_EVENT_KEY", OXY_EVENT_KEY.into()),
        ("OXY_EVENT_FOCUS", OXY_EVENT_FOCUS.into()),
        ("OXY_EVENT_TOUCH", OXY_EVENT_TOUCH.into()),
        ("OXY_EVENT_FEEDBACK", OXY_EVENT_FEEDBACK.into()),
        ("OXY_EVENT_RELEASED", OXY_EVENT_RELEASED.into()),
        ("OXY_EVENT_SCROLL", OXY_EVENT_SCROLL.into()),
        ("OXY_EVENT_TEXT", OXY_EVENT_TEXT.into()),
        ("OXY_EVENT_PREEDIT", OXY_EVENT_PREEDIT.into()),
        ("OXY_POINTER_ENTER", PointerKind::Enter as i64),
        ("OXY_POINTER_LEAVE", PointerKind::Leave as i64),
        ("OXY_POINTER_MOTION", PointerKind::Motion as i64),
        ("OXY_POINTER_PRESS", PointerKind::Press as i64),
        ("OXY_POINTER_RELEASE", PointerKind::Release as i64),
        ("OXY_POINTER_CANCEL", PointerKind::Cancel as i64),
        (
            "OXY_OUTCOME_PRESENTED",
            oxy_protocol::Outcome::Presented as i64,
        ),
        (
            "OXY_OUTCOME_DISCARDED",
            oxy_protocol::Outcome::Discarded as i64,
        ),
        ("OXY_FORMAT_XRGB8888", OXY_FORMAT_XRGB8888.into()),
        ("OXY_FORMAT_ARGB8888", OXY_FORMAT_ARGB8888.into()),
        ("OXY_POLL_EVENT", OXY_POLL_EVENT.into()),
        ("OXY_POLL_EMPTY", OXY_POLL_EMPTY.into()),
        ("OXY_POLL_CLOSED", OXY_POLL_CLOSED.into()),
        ("OXY_ERROR", OXY_ERROR.into()),
    ] {
        assert_eq!(define(name), value, "{name}");
    }

    let event = fields("OxyEvent");
    let rust = [
        offset_of!(OxyEvent, kind),
        offset_of!(OxyEvent, surface),
        offset_of!(OxyEvent, serial),
        offset_of!(OxyEvent, x),
        offset_of!(OxyEvent, y),
        offset_of!(OxyEvent, width),
        offset_of!(OxyEvent, height),
        offset_of!(OxyEvent, scale_120),
        offset_of!(OxyEvent, detail),
        offset_of!(OxyEvent, code),
        offset_of!(OxyEvent, flag),
        offset_of!(OxyEvent, text),
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
    assert_eq!(c_offsets(&event), (rust.to_vec(), size_of::<OxyEvent>()));

    let positioner = fields("OxyPositioner");
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
        offset_of!(OxyPositioner, anchor_x),
        offset_of!(OxyPositioner, anchor_y),
        offset_of!(OxyPositioner, anchor_width),
        offset_of!(OxyPositioner, anchor_height),
        offset_of!(OxyPositioner, width),
        offset_of!(OxyPositioner, height),
        offset_of!(OxyPositioner, anchor),
        offset_of!(OxyPositioner, gravity),
        offset_of!(OxyPositioner, adjust),
        offset_of!(OxyPositioner, offset_x),
        offset_of!(OxyPositioner, offset_y),
    ];
    assert_eq!(
        c_offsets(&positioner),
        (rust.to_vec(), size_of::<OxyPositioner>())
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
        OxyEvent {
            kind: OXY_EVENT_POINTER,
            surface: 1,
            serial: 9,
            x: 2560,
            y: 5120,
            detail: PointerKind::Press as u32,
            code: 272,
            ..OxyEvent::default()
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
        text: oxy_protocol::TextChunk::new("héllo").unwrap(),
    })
    .unwrap();
    assert_eq!(text.kind, OXY_EVENT_TEXT);
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
        (OXY_EVENT_SCROLL, 0, 512)
    );
    assert_eq!(convert(&Event::Welcome { version: 2 }), None);
}

#[test]
fn positioners_with_unknown_values_are_rejected() {
    let valid = OxyPositioner {
        anchor_width: 1,
        anchor_height: 1,
        width: 10,
        height: 10,
        anchor: 8,
        gravity: 8,
        adjust: 15,
        ..OxyPositioner::default()
    };
    assert!(positioner(&valid).is_some());
    assert!(positioner(&OxyPositioner { anchor: 9, ..valid }).is_none());
    assert!(positioner(&OxyPositioner {
        gravity: 200,
        ..valid
    })
    .is_none());
    assert!(positioner(&OxyPositioner {
        adjust: 128,
        ..valid
    })
    .is_none());
}

#[test]
fn a_session_through_the_c_abi() {
    let directory = std::env::temp_dir().join(format!("oxy-client-ffi-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("s");
    let listener = Listener::bind(&socket).unwrap();
    let compositor = std::thread::spawn(move || {
        let server = listener.accept().unwrap();
        let mut protocol = ServerConnection::new();
        let (hello, _) = server.recv_request().unwrap();
        server
            .send_event(&protocol.receive(&hello).unwrap().unwrap())
            .unwrap();
        assert_eq!(
            server.recv_request().unwrap().0,
            Request::CreateToplevel {
                surface: 1,
                size: Size::new(2, 1)
            }
        );
        server
            .send_event(&Event::Configure {
                surface: 1,
                serial: 5,
                rect: Rect::new(0, 0, 2, 1),
                scale_120: 120,
                fullscreen: false,
            })
            .unwrap();
        let (attach, fd) = server.recv_request().unwrap();
        let shared = SharedBuffer::import(fd.unwrap(), attach.buffer_layout().unwrap()).unwrap();
        let (submit, _) = server.recv_request().unwrap();
        let mut row = vec![0; 8];
        shared.read_row(0, &mut row).unwrap();
        (submit, row)
    });

    let path = CString::new(socket.to_str().unwrap()).unwrap();
    // SAFETY: the client pointer comes from oxy_client_connect, is used by this thread only and
    // is closed once at the end; every other pointer is to a live local of the right type.
    unsafe {
        let client = oxy_client_connect(path.as_ptr());
        assert!(!client.is_null());
        assert!(oxy_client_fd(client) >= 0);
        assert_eq!(oxy_client_create_toplevel(client, 1, 2, 1), 0);
        let mut event = OxyEvent::default();
        let configure = loop {
            match oxy_client_poll(client, &mut event) {
                OXY_POLL_EVENT => break event,
                OXY_POLL_EMPTY => std::thread::yield_now(),
                other => panic!("poll returned {other}"),
            }
        };
        assert_eq!((configure.kind, configure.serial), (OXY_EVENT_CONFIGURE, 5));
        assert_eq!(
            (configure.width, configure.height, configure.scale_120),
            (2, 1, 120)
        );
        let pixels = [1u8, 2, 3, 0, 4, 5, 6, 0];
        let present = |width, height| {
            oxy_client_present(
                client,
                1,
                5,
                width,
                height,
                OXY_FORMAT_XRGB8888,
                pixels.as_ptr(),
                pixels.len(),
            )
        };
        assert_eq!(present(2, 1), 1);
        // A frame whose pixels do not match its size is refused, with a reason.
        assert_eq!(present(4, 4), -1);
        let reason = CStr::from_ptr(oxy_last_error()).to_str().unwrap();
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
        assert_eq!(oxy_client_poll(client, &mut event), OXY_POLL_CLOSED);
        oxy_client_close(client);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn null_arguments_fail_instead_of_crashing() {
    // SAFETY: NULL is accepted by every entry point and reported as an error.
    unsafe {
        assert!(oxy_client_connect(std::ptr::null()).is_null());
        assert_eq!(oxy_client_fd(std::ptr::null_mut()), OXY_ERROR);
        assert_eq!(
            oxy_client_create_toplevel(std::ptr::null_mut(), 1, 1, 1),
            OXY_ERROR
        );
        assert_eq!(
            oxy_client_poll(std::ptr::null_mut(), std::ptr::null_mut()),
            OXY_ERROR
        );
        assert_eq!(
            oxy_client_reposition(std::ptr::null_mut(), 2, std::ptr::null()),
            OXY_ERROR
        );
        oxy_client_close(std::ptr::null_mut());
    }
}
