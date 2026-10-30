use super::*;
use std::io::Cursor;

fn positioner() -> Positioner {
    Positioner {
        anchor_rect: Rect::new(10, 20, 30, 40),
        size: Size::new(200, 300),
        anchor: Anchor::BottomLeft,
        gravity: Gravity::BottomRight,
        adjust: Adjust::FLIP_Y.union(Adjust::SLIDE_X),
        offset_x: -4,
        offset_y: 2,
    }
}

fn requests() -> Vec<Request> {
    let size = Size::new(800, 600);
    vec![
        Request::Hello {
            min_version: MIN_VERSION,
            max_version: VERSION,
        },
        Request::CreateToplevel { surface: 1, size },
        Request::CreatePopup {
            surface: 2,
            parent: 1,
            positioner: positioner(),
        },
        Request::CreateDialog {
            surface: 3,
            parent: 1,
            size,
        },
        Request::CreateShellSurface {
            surface: 4,
            kind: ShellKind::Panel,
            edge: PanelEdge::Top,
            output: 1,
            size,
            exclusive: 32,
        },
        Request::CreateLockSurface {
            surface: 5,
            output: 1,
        },
        Request::GrabPopup {
            surface: 2,
            serial: 9,
        },
        Request::Reposition {
            surface: 2,
            positioner: positioner(),
        },
        Request::SetInputRegion {
            surface: 4,
            region: Rect::new(0, 0, 0, 0),
        },
        Request::Submit {
            surface: 1,
            sequence: 1,
            buffer: 2,
            configure: 1,
            extent: size,
        },
        Request::Destroy { surface: 2 },
        Request::BeginMove {
            surface: 1,
            serial: 3,
        },
        Request::SetFullscreen {
            surface: 1,
            enabled: true,
        },
        Request::Goodbye {},
        Request::AttachBuffer {
            buffer: 7,
            format: Format::Xrgb8888,
            extent: size,
            stride: 3200,
            offset: 0,
        },
        Request::DestroyBuffer { buffer: 7 },
        Request::AccessibilityBegin {
            surface: 1,
            nodes: 2,
        },
        Request::AccessibleNode {
            surface: 1,
            index: 1,
            parent: 0,
            role: AccessibleRole::MenuItem,
            state: AccessibleState::new(AccessibleState::FOCUSED).unwrap(),
            actions: AccessibleActions::new(AccessibleActions::DISMISS).unwrap(),
            rect: Rect::new(8, 8, 248, 44),
        },
        Request::AccessibleText {
            surface: 1,
            index: 1,
            field: AccessibleField::Name,
            text: TextChunk::new("Mover a carpeta").unwrap(),
        },
        Request::AccessibilityCommit { surface: 1 },
        Request::AccessibilityAction {
            client: 3,
            surface: 1,
            index: 1,
            action: AccessibleAction::SetSelection,
            custom: 0,
            start: 2,
            end: 5,
        },
        Request::Damage {
            surface: 1,
            rect: Rect::new(-2, 3, 40, 5),
        },
        Request::ClipboardOffer {
            offer: 4,
            serial: 9,
        },
        Request::ClipboardType {
            offer: 4,
            mime: TextChunk::new("text/plain;charset=utf-8").unwrap(),
        },
        Request::ClipboardCommit { offer: 4 },
        Request::ClipboardClear { serial: 10 },
        Request::ClipboardReceive {
            selection: 2,
            mime: TextChunk::new("text/plain;charset=utf-8").unwrap(),
            serial: 11,
        },
        Request::TextInputEnable { surface: 1 },
        Request::TextInputPurpose {
            surface: 1,
            purpose: TextPurpose::Email,
            hints: TextHints::new(TextHints::NO_CAPITALIZATION).unwrap(),
        },
        Request::TextInputSurrounding {
            surface: 1,
            text: TextChunk::new("nate@").unwrap(),
        },
        Request::TextInputSelection {
            surface: 1,
            anchor: 5,
            cursor: 5,
        },
        Request::TextInputCaret {
            surface: 1,
            rect: Rect::new(40, 8, 2, 18),
        },
        Request::TextInputCommit {
            surface: 1,
            serial: 1,
        },
        Request::TextInputDisable { surface: 1 },
        Request::InputMethodKeyHandled {
            serial: 20,
            used: true,
        },
        Request::InputMethodDelete {
            before: 3,
            after: 0,
        },
        Request::InputMethodCommitText {
            text: TextChunk::new("日本").unwrap(),
        },
        Request::InputMethodPreeditText {
            text: TextChunk::new("ご").unwrap(),
        },
        Request::InputMethodPreeditCursor { begin: 0, end: 3 },
        Request::InputMethodPreeditSegment {
            begin: 0,
            end: 3,
            style: PreeditStyle::Converting,
        },
        Request::InputMethodCommit { serial: 2 },
        Request::CreateInputCandidates {
            surface: 3,
            size: Size::new(240, 120),
        },
        Request::SetTitle {
            surface: 1,
            index: 0,
            last: true,
            text: PreeditText::new("Bandeja de entrada").unwrap(),
        },
        Request::SetAppId {
            surface: 1,
            index: 0,
            last: true,
            text: PreeditText::new("so.oxy.inbox").unwrap(),
        },
        Request::FocusWindow { window: 7 },
        Request::CloseWindow { window: 7 },
        Request::MinimizeWindow { window: 7 },
        Request::MoveWindowToWorkspace {
            window: 7,
            workspace: 1,
        },
        Request::ActivateWorkspace { workspace: 1 },
        Request::AddWorkspace {},
        Request::RemoveWorkspace { workspace: 1 },
        Request::SetPointerShape {
            surface: 1,
            shape: PointerShape::ResizeTopRight,
        },
        Request::LaunchApplication {
            serial: 3,
            index: 0,
            last: true,
            text: PreeditText::new("so.oxy.allo").unwrap(),
        },
        Request::LockSession {},
        Request::AuthenticateSecret {
            surface: 5,
            secret: SecretChunk::new("correct horse").unwrap(),
        },
        Request::Authenticate { surface: 5 },
        Request::AccessibleSelection {
            surface: 1,
            index: 1,
            start: 3,
            end: 7,
        },
        Request::AccessibilityAnnounce {
            surface: 1,
            index: 0,
            last: true,
            politeness: Politeness::Assertive,
            text: PreeditText::new("Guardado").unwrap(),
        },
    ]
}

fn events() -> Vec<Event> {
    vec![
        Event::Welcome { version: VERSION },
        Event::Configure {
            surface: 1,
            serial: 2,
            rect: Rect::new(-10, 5, 640, 480),
            scale_120: 180,
            fullscreen: false,
        },
        Event::PopupDone {
            surface: 2,
            reason: DismissReason::OutsidePress,
        },
        Event::Pointer {
            surface: 1,
            serial: 4,
            kind: PointerKind::Press,
            x: -1,
            y: 7,
            button: 272,
        },
        Event::Key {
            surface: 1,
            serial: 5,
            key: 1,
            pressed: true,
        },
        Event::Focus {
            surface: 1,
            focused: false,
        },
        Event::Touch {
            surface: 1,
            serial: 6,
            kind: TouchKind::Cancel,
            id: 0,
            x: 0,
            y: 0,
        },
        Event::Pinch {
            surface: 1,
            serial: 7,
            phase: GesturePhase::Update,
            x: -3,
            y: 4,
            scale_65536: 98304,
            rotation_256: -2560,
        },
        Event::LaunchResult {
            serial: 3,
            launched: true,
        },
        Event::LockState { locked: true },
        Event::CapsLock {
            surface: 5,
            on: true,
        },
        Event::AccessibleSelection {
            client: 3,
            surface: 2,
            index: 1,
            start: 3,
            end: 7,
        },
        Event::AccessibilityAnnouncement {
            client: 3,
            surface: 2,
            index: 0,
            last: true,
            politeness: Politeness::Polite,
            text: PreeditText::new("Guardado").unwrap(),
        },
        Event::AuthenticationResult {
            surface: 5,
            outcome: AuthOutcome::Throttled,
            wait_s: 30,
        },
        Event::Feedback {
            surface: 1,
            sequence: 1,
            outcome: Outcome::Presented,
            timestamp_ns: 99,
            clock: Clock::Synthetic,
        },
        Event::Scroll {
            surface: 1,
            serial: 7,
            dx: 0,
            dy: -256 * 40,
        },
        Event::Text {
            surface: 1,
            serial: 8,
            text: TextChunk::new("héllo").unwrap(),
        },
        Event::Preedit {
            surface: 1,
            serial: 9,
            text: PreeditText::new("にほ").unwrap(),
            cursor: 3,
        },
        Event::Released {
            surface: 1,
            buffer: 2,
        },
        Event::ProtocolError {
            code: ErrorCode::Unauthorized,
        },
        Event::AccessibilityAction {
            surface: 2,
            serial: 12,
            index: 4,
            action: AccessibleAction::Custom,
            custom: 1,
            start: 0,
            end: 0,
        },
        Event::AccessibilityTree {
            client: 3,
            surface: 2,
            parent: 1,
            nodes: 8,
        },
        Event::AccessibleNode {
            client: 3,
            surface: 2,
            index: 0,
            parent: NO_ACCESSIBLE_PARENT,
            role: AccessibleRole::Menu,
            state: AccessibleState::new(AccessibleState::LIVE_POLITE).unwrap(),
            actions: AccessibleActions::default(),
            rect: Rect::new(981, 382, 266, 338),
        },
        Event::AccessibleText {
            client: 3,
            surface: 2,
            index: 0,
            field: AccessibleField::Value,
            text: TextChunk::new("Hola").unwrap(),
        },
        Event::AccessibilityCommitted {
            client: 3,
            surface: 2,
        },
        Event::AccessibilityRemoved {
            client: 3,
            surface: 2,
        },
        Event::ClipboardTypes {
            selection: 2,
            count: 1,
        },
        Event::ClipboardType {
            selection: 2,
            mime: TextChunk::new("text/plain;charset=utf-8").unwrap(),
        },
        Event::ClipboardEmpty {},
        Event::ClipboardSend {
            offer: 4,
            mime: TextChunk::new("text/plain;charset=utf-8").unwrap(),
        },
        Event::TextInputDelete {
            surface: 1,
            before: 3,
            after: 0,
        },
        Event::TextInputCommitText {
            surface: 1,
            text: TextChunk::new("日本").unwrap(),
        },
        Event::TextInputPreeditText {
            surface: 1,
            text: TextChunk::new("ご").unwrap(),
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
            serial: 12,
            commit: 1,
        },
        Event::InputMethodActivate {},
        Event::InputMethodDeactivate {},
        Event::InputMethodPurpose {
            purpose: TextPurpose::Normal,
            hints: TextHints::default(),
        },
        Event::InputMethodSurrounding {
            text: TextChunk::new("nihon").unwrap(),
        },
        Event::InputMethodSelection {
            anchor: 5,
            cursor: 5,
        },
        Event::InputMethodDone { serial: 2 },
        Event::InputMethodKey {
            serial: 20,
            key: 30,
            pressed: true,
            text: PreeditText::new("a").unwrap(),
        },
        Event::WindowState {
            window: 7,
            workspace: 1,
            minimized: false,
            focused: true,
            launched: false,
        },
        Event::WindowTitle {
            window: 7,
            index: 0,
            last: true,
            text: PreeditText::new("Bandeja de entrada").unwrap(),
        },
        Event::WindowAppId {
            window: 7,
            index: 0,
            last: true,
            text: PreeditText::new("so.oxy.inbox").unwrap(),
        },
        Event::WindowClosed { window: 7 },
        Event::Workspaces {
            count: 2,
            active: 1,
        },
        Event::CloseRequested { surface: 1 },
    ]
}

#[test]
fn text_purposes_hints_and_styles_are_closed_sets() {
    assert!(TextHints::new(1 << 4).is_none());
    assert!(TextPurpose::from_raw(11).is_none());
    assert!(PreeditStyle::from_raw(0).is_none());
    let mut bytes = Vec::new();
    write_request(
        &mut bytes,
        &Request::TextInputPurpose {
            surface: 1,
            purpose: TextPurpose::Password,
            hints: TextHints::new(TextHints::SENSITIVE).unwrap(),
        },
    )
    .unwrap();
    // The purpose byte follows the 4-byte length, the opcode and the surface; the hints follow it.
    let purpose = 4 + 1 + 4;
    let mut bad_purpose = bytes.clone();
    bad_purpose[purpose] = 11;
    assert!(read_request(&mut Cursor::new(bad_purpose)).is_err());
    let mut bad_hints = bytes.clone();
    bad_hints[purpose + 1] = 1 << 4;
    assert!(read_request(&mut Cursor::new(bad_hints)).is_err());
    assert!(read_request(&mut Cursor::new(bytes)).is_ok());
}

#[test]
fn accessible_roles_and_states_are_closed_sets() {
    assert!(AccessibleState::new(1 << 10).is_none());
    assert!(AccessibleActions::new(1 << 6).is_none());
    assert!(AccessibleState::new(AccessibleState::EXPANDED | AccessibleState::FOCUSED).is_some());
    let mut bytes = Vec::new();
    write_request(
        &mut bytes,
        &Request::AccessibleNode {
            surface: 1,
            index: 0,
            parent: NO_ACCESSIBLE_PARENT,
            role: AccessibleRole::Button,
            state: AccessibleState::default(),
            actions: AccessibleActions::default(),
            rect: Rect::new(0, 0, 10, 10),
        },
    )
    .unwrap();
    // The role byte follows the 4-byte length, the opcode and three u32 fields.
    let role = 4 + 1 + 12;
    let mut bad_role = bytes.clone();
    bad_role[role] = 200;
    assert!(read_request(&mut Cursor::new(bad_role)).is_err());
    let mut bad_state = bytes.clone();
    bad_state[role + 2] = 0xff;
    assert!(read_request(&mut Cursor::new(bad_state)).is_err());
    assert!(read_request(&mut Cursor::new(bytes)).is_ok());
}

#[test]
fn every_message_round_trips_without_consuming_the_next_packet() {
    let mut bytes = Vec::new();
    for request in requests() {
        write_request(&mut bytes, &request).unwrap();
    }
    let mut cursor = Cursor::new(bytes);
    for request in requests() {
        assert_eq!(read_request(&mut cursor).unwrap(), request);
    }
    assert_eq!(cursor.position(), cursor.get_ref().len() as u64);

    let mut bytes = Vec::new();
    for event in events() {
        write_event(&mut bytes, &event).unwrap();
    }
    let mut cursor = Cursor::new(bytes);
    for event in events() {
        assert_eq!(read_event(&mut cursor).unwrap(), event);
    }
    assert_eq!(requests().len(), Request::SCHEMA.len());
    assert_eq!(events().len(), Event::SCHEMA.len());
}

#[test]
fn a_secret_is_never_printed() {
    let request = Request::AuthenticateSecret {
        surface: 5,
        secret: SecretChunk::new("correct horse").unwrap(),
    };
    assert!(!format!("{request:?}").contains("horse"));
    assert_eq!(
        SecretChunk::split(&"é".repeat(20))
            .iter()
            .map(SecretChunk::as_str)
            .collect::<String>(),
        "é".repeat(20)
    );
}

#[test]
fn schema_table_matches_documentation() {
    let doc = include_str!("../../../docs/native-foundation.mdx");
    for (direction, schema) in [("Request", Request::SCHEMA), ("Event", Event::SCHEMA)] {
        for (opcode, name, length) in schema {
            let row = format!("| {direction} | {opcode} | {name} | {length} |");
            assert!(doc.contains(&row), "documentation lacks `{row}`");
        }
    }
}

#[test]
fn directions_have_separate_decoders() {
    // A compositor event replayed by a client is not a request, and vice versa.
    let mut welcome = Vec::new();
    write_event(&mut welcome, &Event::Welcome { version: VERSION }).unwrap();
    assert!(read_request(&mut Cursor::new(welcome)).is_err());
    let mut destroy = Vec::new();
    write_request(&mut destroy, &Request::Destroy { surface: 1 }).unwrap();
    assert!(read_event(&mut Cursor::new(destroy)).is_err());
}

#[test]
fn rejects_unbounded_lengths_before_reading_the_payload() {
    for length in [0_u32, MAX_PACKET_BYTES as u32 + 1, u32::MAX] {
        let error = read_request(&mut Cursor::new(length.to_le_bytes())).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}

#[test]
fn rejects_invalid_field_values() {
    let bad = |body: Vec<u8>| {
        let mut packet = (body.len() as u32).to_le_bytes().to_vec();
        packet.extend(body);
        read_request(&mut Cursor::new(packet)).is_err()
    };
    // Unknown opcode, wrong exact length.
    assert!(bad(vec![99]));
    assert!(bad(vec![14, 0]));
    // Zero-sized toplevel and oversized dimension.
    let mut body = vec![2, 1, 0, 0, 0];
    body.extend(0_u32.to_le_bytes());
    body.extend(1_u32.to_le_bytes());
    assert!(bad(body));
    let mut body = vec![2, 1, 0, 0, 0];
    body.extend((MAX_DIMENSION + 1).to_le_bytes());
    body.extend(1_u32.to_le_bytes());
    assert!(bad(body));
    // Boolean other than 0/1.
    assert!(bad(vec![13, 1, 0, 0, 0, 2]));
    // Unknown shell kind.
    let mut body = vec![5, 1, 0, 0, 0, 9, 0];
    body.extend(1_u32.to_le_bytes());
    body.extend(1_u32.to_le_bytes());
    body.extend(1_u32.to_le_bytes());
    body.extend(0_u32.to_le_bytes());
    assert!(bad(body));
    // Encoders refuse what decoders refuse.
    let mut sink = Vec::new();
    let mut popup = positioner();
    popup.anchor_rect.width = 0;
    assert!(write_request(
        &mut sink,
        &Request::CreatePopup {
            surface: 1,
            parent: 2,
            positioner: popup
        }
    )
    .is_err());
    assert!(sink.is_empty());
}

#[test]
fn reads_fragmented_streams() {
    struct Fragmented(Cursor<Vec<u8>>);
    impl Read for Fragmented {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            let count = bytes.len().min(1);
            self.0.read(&mut bytes[..count])
        }
    }
    let mut bytes = Vec::new();
    for request in requests() {
        write_request(&mut bytes, &request).unwrap();
    }
    let mut stream = Fragmented(Cursor::new(bytes));
    for request in requests() {
        assert_eq!(read_request(&mut stream).unwrap(), request);
    }
}

#[test]
fn every_truncation_of_every_message_is_an_error() {
    for request in requests() {
        let mut bytes = Vec::new();
        write_request(&mut bytes, &request).unwrap();
        for cut in 0..bytes.len() {
            let error = read_request(&mut Cursor::new(&bytes[..cut])).unwrap_err();
            assert_eq!(
                error.kind(),
                io::ErrorKind::UnexpectedEof,
                "{request:?} at {cut}"
            );
        }
        // A header that claims a shorter or longer body than the message is rejected.
        for delta in [-1_i64, 1] {
            let mut altered = bytes.clone();
            let length = (bytes.len() as i64 - 4 + delta) as u32;
            altered[..4].copy_from_slice(&length.to_le_bytes());
            altered.push(0);
            assert!(read_request(&mut Cursor::new(altered)).is_err());
        }
    }
}

/// Deterministic xorshift so the fuzz corpus is reproducible without external crates.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn fuzzed_streams_never_panic_and_only_yield_valid_messages() {
    let mut rng = Rng(0x5eed_0a11_2026_0030);
    let mut seeds = Vec::new();
    for request in requests() {
        let mut bytes = Vec::new();
        write_request(&mut bytes, &request).unwrap();
        seeds.push(bytes);
    }
    let mut decoded = 0;
    for round in 0..200_000 {
        let mut bytes = seeds[round % seeds.len()].clone();
        match rng.next() % 4 {
            0 => {
                // Flip random bits, including the header.
                for _ in 0..1 + rng.next() % 4 {
                    let index = (rng.next() as usize) % bytes.len();
                    bytes[index] ^= 1 << (rng.next() % 8);
                }
            }
            1 => bytes.truncate((rng.next() as usize) % bytes.len()),
            2 => {
                let length = (rng.next() % 80) as usize;
                bytes = (0..length).map(|_| rng.next() as u8).collect();
            }
            _ => bytes.extend((0..rng.next() % 8).map(|_| rng.next() as u8)),
        }
        let mut cursor = Cursor::new(bytes);
        while let Ok(request) = read_request(&mut cursor) {
            // Anything decoded must re-encode to an identical, accepted packet.
            let mut again = Vec::new();
            write_request(&mut again, &request).unwrap();
            assert_eq!(read_request(&mut Cursor::new(again)).unwrap(), request);
            decoded += 1;
        }
    }
    assert!(decoded > 0, "the corpus should still contain valid packets");
}

#[test]
fn version_negotiation_selects_the_highest_common_version() {
    assert_eq!(negotiate(MIN_VERSION, u16::MAX), Ok(VERSION));
    assert_eq!(negotiate(1, 1), Err(ErrorCode::UnsupportedVersion));
    assert_eq!(
        negotiate(VERSION + 1, VERSION + 3),
        Err(ErrorCode::UnsupportedVersion)
    );
    assert_eq!(negotiate(3, 2), Err(ErrorCode::InvalidMessage));
}

#[test]
fn server_state_machine_rejects_invalid_order() {
    let destroy = Request::Destroy { surface: 1 };
    let hello = Request::Hello {
        min_version: MIN_VERSION,
        max_version: VERSION,
    };

    let mut connection = ServerConnection::new();
    assert_eq!(connection.receive(&destroy), Err(ErrorCode::InvalidOrder));
    assert!(connection.is_closed());
    assert_eq!(connection.receive(&hello), Err(ErrorCode::InvalidOrder));

    let mut connection = ServerConnection::new();
    assert_eq!(
        connection.receive(&hello),
        Ok(Some(Event::Welcome { version: VERSION }))
    );
    assert_eq!(connection.version(), Some(VERSION));
    assert_eq!(connection.receive(&destroy), Ok(None));
    assert_eq!(connection.receive(&hello), Err(ErrorCode::InvalidOrder));
    assert!(connection.is_closed());

    let mut connection = ServerConnection::new();
    connection.receive(&hello).unwrap();
    connection.receive(&Request::Goodbye {}).unwrap();
    assert_eq!(connection.receive(&destroy), Err(ErrorCode::InvalidOrder));

    let mut connection = ServerConnection::new();
    assert_eq!(
        connection.receive(&Request::Hello {
            min_version: 1,
            max_version: 1
        }),
        Err(ErrorCode::UnsupportedVersion)
    );
    assert!(connection.is_closed());
}

#[test]
fn client_state_machine_requires_a_requested_welcome_first() {
    let (mut client, hello) = ClientConnection::new(MIN_VERSION, VERSION);
    assert!(matches!(hello, Request::Hello { .. }));
    assert_eq!(
        client.receive(&Event::Focus {
            surface: 1,
            focused: true
        }),
        Err(ErrorCode::InvalidOrder)
    );

    let (mut client, _) = ClientConnection::new(MIN_VERSION, VERSION);
    assert_eq!(
        client.receive(&Event::Welcome {
            version: VERSION + 1
        }),
        Err(ErrorCode::InvalidOrder)
    );

    let (mut client, _) = ClientConnection::new(MIN_VERSION, VERSION);
    client
        .receive(&Event::Welcome { version: VERSION })
        .unwrap();
    assert_eq!(client.version(), Some(VERSION));
    assert_eq!(
        client.receive(&Event::Welcome { version: VERSION }),
        Err(ErrorCode::InvalidOrder)
    );
}

#[test]
fn buffer_layouts_are_checked_without_overflow() {
    let layout = |width, height, stride, offset| BufferLayout {
        format: Format::Argb8888,
        extent: Size::new(width, height),
        stride,
        offset,
    };
    assert_eq!(layout(800, 600, 3200, 0).required_bytes(), Some(3200 * 600));
    // The last row only needs its pixels, not the full stride.
    assert_eq!(layout(10, 2, 64, 8).required_bytes(), Some(8 + 64 + 40));
    assert_eq!(
        layout(800, 600, 3199, 0).required_bytes(),
        None,
        "stride shorter than a row"
    );
    assert_eq!(
        layout(800, 600, 3202, 0).required_bytes(),
        None,
        "misaligned stride"
    );
    assert_eq!(
        layout(800, 600, 3200, 2).required_bytes(),
        None,
        "misaligned offset"
    );
    assert_eq!(
        layout(16_384, 16_384, u32::MAX - 3, u32::MAX - 3).required_bytes(),
        None
    );
    assert_eq!(
        layout(16_384, 16_384, 65_536, 0).required_bytes(),
        None,
        "over the byte cap"
    );
    assert!(Request::AttachBuffer {
        buffer: 1,
        format: Format::Xrgb8888,
        extent: Size::new(1, 1),
        stride: 4,
        offset: 0
    }
    .carries_fd());
    assert!(!Request::Goodbye {}.carries_fd());
}

#[test]
fn text_chunks_hold_whole_characters_of_valid_utf8() {
    assert!(TextChunk::new("").is_none());
    assert!(TextChunk::new(&"a".repeat(TEXT_CHUNK_BYTES + 1)).is_none());
    // Splitting never cuts a character: 11 three-byte characters are 33 bytes.
    let text = "€".repeat(11);
    let chunks = TextChunk::split(&text);
    assert_eq!(
        chunks.iter().map(|c| c.as_str().len()).collect::<Vec<_>>(),
        [30, 3]
    );
    assert_eq!(chunks.iter().map(|c| c.as_str()).collect::<String>(), text);

    let encoded = |text: TextChunk| {
        let mut out = Vec::new();
        write_event(
            &mut out,
            &Event::Text {
                surface: 1,
                serial: 1,
                text,
            },
        )
        .unwrap();
        out
    };
    let valid = encoded(TextChunk::new("ok").unwrap());
    // Text starts after the length prefix, opcode, surface and serial.
    let text_at = 4 + 1 + 4 + 8;
    for (offset, value, reason) in [
        (text_at, 0, "zero length"),
        (text_at, 33, "length beyond the chunk"),
        (text_at + 1, 0xff, "invalid UTF-8"),
        (text_at + 1 + 2, b'x', "nonzero padding"),
    ] {
        let mut packet = valid.clone();
        packet[offset] = value;
        assert!(read_event(&mut Cursor::new(packet)).is_err(), "{reason}");
    }
}

#[test]
fn an_empty_preedit_ends_the_composition_and_has_one_encoding() {
    let end = Event::Preedit {
        surface: 1,
        serial: 1,
        text: PreeditText::new("").unwrap(),
        cursor: 0,
    };
    let mut packet = Vec::new();
    write_event(&mut packet, &end).unwrap();
    assert_eq!(read_event(&mut Cursor::new(packet.clone())).unwrap(), end);
    // An empty preedit's padding must be zero, like a text chunk's.
    let text_at = 4 + 1 + 4 + 8;
    packet[text_at + 5] = b'x';
    assert!(read_event(&mut Cursor::new(packet)).is_err());
}
