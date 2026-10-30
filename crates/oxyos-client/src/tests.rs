use super::*;
use oxyos_ipc::SharedBuffer;
use oxyos_protocol::{
    PreeditStyle, ServerConnection, TextHints, TextPurpose, MAX_SURROUNDING_BYTES,
};

/// A connected client and the compositor's end, past the handshake.
fn connected() -> (Client, Connection) {
    let (client, server) = Connection::pair().unwrap();
    let compositor = std::thread::spawn(move || {
        let mut protocol = ServerConnection::new();
        let (hello, _) = server.recv_request().unwrap();
        let welcome = protocol.receive(&hello).unwrap().expect("welcome");
        server.send_event(&welcome, None).unwrap();
        server
    });
    let client = Client::handshake(client).unwrap();
    (client, compositor.join().unwrap())
}

fn pixels(extent: Size, value: u8) -> Vec<u8> {
    vec![value; (extent.width * extent.height * 4) as usize]
}

/// Receives an AttachBuffer and its descriptor, then the Submit naming it.
fn submitted(server: &Connection) -> (u64, SharedBuffer, Request) {
    let (attach, fd) = server.recv_request().unwrap();
    let (Request::AttachBuffer { buffer, .. }, Some(fd)) = (attach, fd) else {
        panic!("expected AttachBuffer, got {attach:?}");
    };
    let shared = SharedBuffer::import(fd, attach.buffer_layout().unwrap()).unwrap();
    (buffer, shared, server.recv_request().unwrap().0)
}

#[test]
fn a_frame_carries_the_pixels_in_a_sealed_buffer() {
    let (mut client, server) = connected();
    let extent = Size::new(4, 2);
    let sequence = client.present(1, 7, extent, &pixels(extent, 0x5a)).unwrap();
    let (buffer, shared, submit) = submitted(&server);
    assert_eq!(
        submit,
        Request::Submit {
            surface: 1,
            sequence,
            buffer,
            configure: 7,
            extent
        }
    );
    let mut row = vec![0; 16];
    shared.read_row(1, &mut row).unwrap();
    assert_eq!(row, vec![0x5a; 16]);
}

#[test]
fn released_buffers_are_reused_and_busy_ones_are_not() {
    let (mut client, server) = connected();
    let extent = Size::new(2, 2);
    client.present(1, 1, extent, &pixels(extent, 1)).unwrap();
    let (first, _, _) = submitted(&server);
    // The first buffer is still held: a second frame needs a second buffer.
    client.present(1, 1, extent, &pixels(extent, 2)).unwrap();
    let (second, _, _) = submitted(&server);
    assert_ne!(first, second);
    server
        .send_event(
            &Event::Released {
                surface: 1,
                buffer: first,
            },
            None,
        )
        .unwrap();
    assert!(matches!(
        client.poll().unwrap(),
        Poll::Event(Event::Released { .. })
    ));
    client.present(1, 1, extent, &pixels(extent, 3)).unwrap();
    // No new AttachBuffer: the released buffer is submitted again.
    match server.recv_request().unwrap().0 {
        Request::Submit { buffer, .. } => assert_eq!(buffer, first),
        other => panic!("expected Submit, got {other:?}"),
    }
    assert_eq!(client.buffer_count(), 2);
}

#[test]
fn a_resize_destroys_idle_buffers_of_the_old_size() {
    let (mut client, server) = connected();
    let small = Size::new(2, 2);
    client.present(1, 1, small, &pixels(small, 1)).unwrap();
    let (old, _, _) = submitted(&server);
    server
        .send_event(
            &Event::Released {
                surface: 1,
                buffer: old,
            },
            None,
        )
        .unwrap();
    client.poll().unwrap();
    let large = Size::new(3, 3);
    client.present(1, 2, large, &pixels(large, 1)).unwrap();
    assert_eq!(
        server.recv_request().unwrap().0,
        Request::DestroyBuffer { buffer: old }
    );
    submitted(&server);
    assert_eq!(client.buffer_count(), 1);
}

#[test]
fn pixels_must_match_the_extent() {
    let (mut client, _server) = connected();
    let error = client
        .present(1, 1, Size::new(4, 4), &pixels(Size::new(2, 2), 0))
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(client.buffer_count(), 0);
}

#[test]
fn destroying_a_surface_destroys_its_buffers() {
    let (mut client, server) = connected();
    let extent = Size::new(2, 2);
    client.present(2, 1, extent, &pixels(extent, 1)).unwrap();
    let (buffer, _, _) = submitted(&server);
    client.destroy_surface(2).unwrap();
    assert_eq!(
        server.recv_request().unwrap().0,
        Request::Destroy { surface: 2 }
    );
    assert_eq!(
        server.recv_request().unwrap().0,
        Request::DestroyBuffer { buffer }
    );
    assert_eq!(client.buffer_count(), 0);
}

#[test]
fn polling_reports_empty_and_closed_and_protocol_errors_end_the_connection() {
    let (mut client, server) = connected();
    assert_eq!(client.poll().unwrap(), Poll::Empty);
    server
        .send_event(
            &Event::ProtocolError {
                code: oxyos_protocol::ErrorCode::InvalidBuffer,
            },
            None,
        )
        .unwrap();
    assert!(client.poll().is_err());
    drop(server);
    assert_eq!(client.poll().unwrap(), Poll::Closed);
}

#[test]
fn a_compositor_that_closes_with_requests_unread_is_a_closed_connection() {
    let (mut client, server) = connected();
    client.send(&Request::Destroy { surface: 9 }).unwrap();
    drop(server);
    assert_eq!(client.poll().unwrap(), Poll::Closed);
    assert_eq!(client.next_event().unwrap(), None);
}

#[test]
fn a_frame_in_another_format_takes_a_buffer_of_that_format() {
    let (mut client, server) = connected();
    let extent = Size::new(2, 2);
    client.present(1, 1, extent, &pixels(extent, 1)).unwrap();
    let (opaque, _, _) = submitted(&server);
    server
        .send_event(
            &Event::Released {
                surface: 1,
                buffer: opaque,
            },
            None,
        )
        .unwrap();
    client.poll().unwrap();
    client
        .present_format(1, 1, extent, Format::Argb8888, &pixels(extent, 2))
        .unwrap();
    assert_eq!(
        server.recv_request().unwrap().0,
        Request::DestroyBuffer { buffer: opaque }
    );
    let (attach, fd) = server.recv_request().unwrap();
    assert!(fd.is_some());
    assert!(matches!(
        attach,
        Request::AttachBuffer {
            format: Format::Argb8888,
            ..
        }
    ));
}

#[test]
fn an_accessibility_snapshot_is_sent_in_order_with_its_text_bounded() {
    let (client, server) = connected();
    let long = "ñ".repeat(600); // 1200 bytes: cut to 1024 at a character boundary
    let nodes = [
        Accessible {
            parent: oxyos_protocol::NO_ACCESSIBLE_PARENT,
            role: AccessibleRole::Menu,
            state: AccessibleState::default(),
            actions: AccessibleActions::new(AccessibleActions::DISMISS).unwrap(),
            rect: Rect::new(0, 0, 200, 100),
            name: "Message actions",
            value: "",
            custom: "Archivar\nResponder",
            selection: None,
        },
        Accessible {
            parent: 0,
            role: AccessibleRole::TextInput,
            state: AccessibleState::new(AccessibleState::FOCUSED).unwrap(),
            actions: AccessibleActions::new(AccessibleActions::SET_SELECTION).unwrap(),
            rect: Rect::new(8, 8, 184, 40),
            name: "Reply",
            value: &long,
            custom: "",
            // Past the value as it is sent, and backwards: kept within it, in order.
            selection: Some((u32::MAX, 3)),
        },
    ];
    client.publish_accessibility(3, &nodes).unwrap();
    let mut requests = Vec::new();
    loop {
        let (request, _) = server.recv_request().unwrap();
        let done = matches!(request, Request::AccessibilityCommit { .. });
        requests.push(request);
        if done {
            break;
        }
    }
    assert_eq!(
        requests[0],
        Request::AccessibilityBegin {
            surface: 3,
            nodes: 2
        }
    );
    assert!(matches!(
        requests[1],
        Request::AccessibleNode {
            surface: 3,
            index: 0,
            role: AccessibleRole::Menu,
            ..
        }
    ));
    let mut value = String::new();
    for request in &requests {
        if let Request::AccessibleText {
            index: 1,
            field: AccessibleField::Value,
            text,
            ..
        } = request
        {
            value.push_str(text.as_str());
        }
    }
    assert_eq!(value, "ñ".repeat(512));
    // At characters: byte 3 is inside the second ñ.
    assert!(requests.contains(&Request::AccessibleSelection {
        surface: 3,
        index: 1,
        start: 2,
        end: 1024,
    }));
    let labels: String = requests
        .iter()
        .filter_map(|request| match request {
            Request::AccessibleText {
                index: 0,
                field: AccessibleField::Actions,
                text,
                ..
            } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(labels, "Archivar\nResponder");
    let too_many = vec![nodes[0]; MAX_ACCESSIBLE_NODES as usize + 1];
    assert_eq!(
        client
            .publish_accessibility(3, &too_many)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
}

/// The next Submit, past any Damage before it.
fn until_submit(server: &Connection) -> Request {
    loop {
        let request = server.recv_request().unwrap().0;
        match request {
            Request::Damage { .. } => continue,
            Request::Submit { .. } => return request,
            other => panic!("expected Damage or Submit, got {other:?}"),
        }
    }
}

/// The XRGB pixels of a buffer, row by row.
fn contents(shared: &SharedBuffer, extent: Size) -> Vec<u8> {
    let mut all = Vec::new();
    for y in 0..extent.height {
        let mut row = vec![0; extent.width as usize * 4];
        shared.read_row(y, &mut row).unwrap();
        all.extend(row);
    }
    all
}

/// `base` with `rect` set to `value`.
fn painted(base: &[u8], extent: Size, rect: Rect, value: u8) -> Vec<u8> {
    let mut out = base.to_vec();
    for y in rect.y as u32..rect.y as u32 + rect.height {
        for x in rect.x as u32..rect.x as u32 + rect.width {
            let at = ((y * extent.width + x) * 4) as usize;
            out[at..at + 4].fill(value);
        }
    }
    out
}

#[test]
fn damage_reaches_the_compositor_before_its_submit() {
    let (mut client, server) = connected();
    let extent = Size::new(8, 4);
    client.present(1, 1, extent, &pixels(extent, 0)).unwrap();
    submitted(&server);
    // Clipped to the buffer; one past it is dropped.
    let damage = [
        Rect::new(6, 2, 5, 5),
        Rect::new(1, 1, 2, 1),
        Rect::new(9, 0, 1, 1),
    ];
    client
        .present_damaged(
            1,
            1,
            extent,
            Format::Xrgb8888,
            &pixels(extent, 1),
            Some(&damage),
        )
        .unwrap();
    let (attach, _) = server.recv_request().unwrap();
    assert!(
        matches!(attach, Request::AttachBuffer { .. }),
        "a second buffer: the first is busy"
    );
    let requests: Vec<Request> = (0..3).map(|_| server.recv_request().unwrap().0).collect();
    assert_eq!(
        requests[..2],
        [
            Request::Damage {
                surface: 1,
                rect: Rect::new(6, 2, 2, 2)
            },
            Request::Damage {
                surface: 1,
                rect: Rect::new(1, 1, 2, 1)
            }
        ]
    );
    assert!(matches!(requests[2], Request::Submit { .. }));
}

#[test]
fn a_reused_buffer_receives_only_what_changed_since_it_was_written() {
    let (mut client, server) = connected();
    let extent = Size::new(8, 4);
    let xrgb = Format::Xrgb8888;
    let first = pixels(extent, 0);
    client.present(1, 1, extent, &first).unwrap();
    let (a, shared_a, _) = submitted(&server);
    // A second frame changes one corner, into a second buffer (the first is busy).
    let corner = Rect::new(0, 0, 2, 2);
    let second = painted(&first, extent, corner, 2);
    client
        .present_damaged(1, 1, extent, xrgb, &second, Some(&[corner]))
        .unwrap();
    let (attach, _) = server.recv_request().unwrap();
    let Request::AttachBuffer { buffer: b, .. } = attach else {
        panic!("expected AttachBuffer, got {attach:?}");
    };
    until_submit(&server);
    server
        .send_event(
            &Event::Released {
                surface: 1,
                buffer: a,
            },
            None,
        )
        .unwrap();
    client.poll().unwrap();
    // The third frame changes another corner, and (against what it promises) everything else
    // too: only what the first buffer missed is written into it, the two corners.
    let other = Rect::new(6, 2, 2, 2);
    let mut third = painted(&second, extent, other, 3);
    for (index, byte) in third.iter_mut().enumerate() {
        let (x, y) = (
            (index / 4) as u32 % extent.width,
            (index / 4) as u32 / extent.width,
        );
        let inside = |rect: Rect| {
            (rect.x as u32..rect.x as u32 + rect.width).contains(&x)
                && (rect.y as u32..rect.y as u32 + rect.height).contains(&y)
        };
        if !inside(corner) && !inside(other) {
            *byte = 9;
        }
    }
    client
        .present_damaged(1, 1, extent, xrgb, &third, Some(&[other]))
        .unwrap();
    let submit = until_submit(&server);
    assert!(
        matches!(submit, Request::Submit { buffer, .. } if buffer == a),
        "{submit:?}"
    );
    let expected = painted(&painted(&first, extent, corner, 2), extent, other, 3);
    assert_eq!(contents(&shared_a, extent), expected);
    assert_ne!(a, b);
}

#[test]
fn without_damage_or_history_a_buffer_is_written_whole() {
    let (mut client, server) = connected();
    let extent = Size::new(4, 2);
    let xrgb = Format::Xrgb8888;
    client.present(1, 1, extent, &pixels(extent, 0)).unwrap();
    let (a, shared_a, _) = submitted(&server);
    // A frame without damage changed everything: the buffer that missed it is written whole.
    client.present(1, 1, extent, &pixels(extent, 1)).unwrap();
    submitted(&server);
    server
        .send_event(
            &Event::Released {
                surface: 1,
                buffer: a,
            },
            None,
        )
        .unwrap();
    client.poll().unwrap();
    let corner = [Rect::new(0, 0, 1, 1)];
    client
        .present_damaged(1, 1, extent, xrgb, &pixels(extent, 7), Some(&corner))
        .unwrap();
    until_submit(&server);
    assert_eq!(contents(&shared_a, extent), pixels(extent, 7));
}

// ---- text input (ADR 0008) -------------------------------------------------------------------

/// The requests up to and including the next TextInputCommit.
fn committed(server: &Connection) -> Vec<Request> {
    let mut requests = Vec::new();
    loop {
        let (request, _) = server.recv_request().unwrap();
        let done = matches!(request, Request::TextInputCommit { .. });
        requests.push(request);
        if done {
            return requests;
        }
    }
}

fn field(text: &str, cursor: usize) -> Field<'_> {
    Field {
        purpose: TextPurpose::Normal,
        hints: TextHints::default(),
        text,
        anchor: cursor,
        cursor,
        caret: Some(Rect::new(4, 2, 2, 18)),
    }
}

#[test]
fn a_field_is_told_whole_then_only_what_changed() {
    let (mut client, server) = connected();
    assert_eq!(
        client
            .text_input(1, Some(&field("hola", 4)), false)
            .unwrap(),
        1
    );
    let chunk = |text| TextChunk::new(text).unwrap();
    assert_eq!(
        committed(&server),
        [
            Request::TextInputEnable { surface: 1 },
            Request::TextInputPurpose {
                surface: 1,
                purpose: TextPurpose::Normal,
                hints: TextHints::default(),
            },
            Request::TextInputSurrounding {
                surface: 1,
                text: chunk("hola"),
            },
            Request::TextInputSelection {
                surface: 1,
                anchor: 4,
                cursor: 4,
            },
            Request::TextInputCaret {
                surface: 1,
                rect: Rect::new(4, 2, 2, 18),
            },
            Request::TextInputCommit {
                surface: 1,
                serial: 1,
            },
        ]
    );
    // Unchanged: nothing is sent, and the serial stays.
    assert_eq!(
        client
            .text_input(1, Some(&field("hola", 4)), false)
            .unwrap(),
        1
    );
    // The caret moved: its selection and rectangle only.
    let moved = Field {
        caret: Some(Rect::new(1, 2, 2, 18)),
        ..field("hola", 1)
    };
    assert_eq!(client.text_input(1, Some(&moved), false).unwrap(), 2);
    assert_eq!(
        committed(&server),
        [
            Request::TextInputSelection {
                surface: 1,
                anchor: 1,
                cursor: 1,
            },
            Request::TextInputCaret {
                surface: 1,
                rect: Rect::new(1, 2, 2, 18),
            },
            Request::TextInputCommit {
                surface: 1,
                serial: 2,
            },
        ]
    );
    // Another field of the surface is enabled anew; disabling commits once.
    assert_eq!(client.text_input(1, Some(&moved), true).unwrap(), 3);
    assert!(matches!(
        committed(&server).as_slice(),
        [
            Request::TextInputEnable { surface: 1 },
            ..,
            Request::TextInputCommit { serial: 3, .. }
        ]
    ));
    assert_eq!(client.text_input(1, None, false).unwrap(), 0);
    assert_eq!(
        committed(&server),
        [
            Request::TextInputDisable { surface: 1 },
            Request::TextInputCommit {
                surface: 1,
                serial: 4,
            },
        ]
    );
    assert_eq!(client.text_input(1, None, false).unwrap(), 0);
    // Serials keep going up across enables.
    assert_eq!(client.text_input(1, Some(&moved), false).unwrap(), 5);
}

#[test]
fn a_long_text_is_told_around_its_selection() {
    let (mut client, server) = connected();
    // 6,000 bytes of two-byte characters, the caret in the middle.
    let text = "ñ".repeat(3000);
    client
        .text_input(1, Some(&field(&text, 3000)), false)
        .unwrap();
    let mut told = String::new();
    let mut selection = None;
    for request in committed(&server) {
        match request {
            Request::TextInputSurrounding { text, .. } => told.push_str(text.as_str()),
            Request::TextInputSelection { anchor, cursor, .. } => {
                selection = Some((anchor, cursor))
            }
            _ => {}
        }
    }
    assert_eq!(told.len(), MAX_SURROUNDING_BYTES);
    assert!(told.chars().all(|c| c == 'ñ'));
    // The caret is told where it is within that part: in its middle.
    assert_eq!(selection, Some((2000, 2000)));
}

#[test]
fn a_transactions_parts_are_collected_until_done() {
    let (mut client, server) = connected();
    let chunk = |text| TextChunk::new(text).unwrap();
    for event in [
        Event::TextInputDelete {
            surface: 1,
            before: 3,
            after: 0,
        },
        Event::TextInputCommitText {
            surface: 1,
            text: chunk(&"日".repeat(10)),
        },
        Event::TextInputCommitText {
            surface: 1,
            text: chunk("本"),
        },
        Event::TextInputPreeditText {
            surface: 1,
            text: chunk("ご"),
        },
        Event::TextInputPreeditCursor {
            surface: 1,
            begin: 0,
            end: 3,
        },
        Event::TextInputPreeditSegment {
            surface: 1,
            begin: 0,
            end: 3,
            style: PreeditStyle::Converting,
        },
        Event::TextInputDone {
            surface: 1,
            serial: 9,
            commit: 2,
        },
        Event::Focus {
            surface: 1,
            focused: false,
        },
    ] {
        server.send_event(&event, None).unwrap();
    }
    let mut completed = Vec::new();
    for _ in 0..8 {
        client.next_event().unwrap();
        completed.push(client.transaction().cloned());
    }
    // Only Done completes it, and the next event drops it.
    assert!(completed[..6].iter().all(Option::is_none));
    assert_eq!(
        completed[6],
        Some(Transaction {
            surface: 1,
            delete_before: 3,
            delete_after: 0,
            commit: format!("{}本", "日".repeat(10)),
            preedit: "ご".to_string(),
            cursor: (0, 3),
            segments: vec![(0, 3, PreeditStyle::Converting)],
            serial: 9,
            commit_serial: 2,
        })
    );
    assert_eq!(completed[7], None);
}

#[test]
fn an_announcement_is_sent_in_chunks_within_its_limit() {
    let (client, server) = connected();
    let long = "é".repeat(600); // 1200 bytes: cut to 1024 at a character boundary
    client
        .announce(4, oxyos_protocol::Politeness::Assertive, &long)
        .unwrap();
    let mut text = String::new();
    let mut indices = Vec::new();
    loop {
        let (request, _) = server.recv_request().unwrap();
        let Request::AccessibilityAnnounce {
            surface: 4,
            index,
            last,
            politeness: oxyos_protocol::Politeness::Assertive,
            text: chunk,
        } = request
        else {
            panic!("{request:?}");
        };
        indices.push(index);
        text.push_str(chunk.as_str());
        if last {
            break;
        }
    }
    assert_eq!(text, "é".repeat(512));
    assert_eq!(indices, (0..32).collect::<Vec<u8>>());
    // Nothing to say, nothing sent: the next request is the one after it.
    client
        .announce(4, oxyos_protocol::Politeness::Polite, "")
        .unwrap();
    client.send(&Request::Goodbye {}).unwrap();
    assert_eq!(server.recv_request().unwrap().0, Request::Goodbye {});
}
