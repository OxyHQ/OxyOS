//! Displays' C ABI (ADR 0018): the constants against the header, events field by field, and
//! Settings' changes sent through a socket as the protocol's requests.

use super::*;

#[test]
fn the_header_matches_the_displays_abi() {
    for (name, value) in [
        ("OXYOS_EVENT_DISPLAY_NAME", OXYOS_EVENT_DISPLAY_NAME),
        ("OXYOS_EVENT_DISPLAY_MODE", OXYOS_EVENT_DISPLAY_MODE),
        ("OXYOS_EVENT_DISPLAY_SCALE", OXYOS_EVENT_DISPLAY_SCALE),
        ("OXYOS_EVENT_DISPLAY", OXYOS_EVENT_DISPLAY),
        ("OXYOS_EVENT_DISPLAY_REMOVED", OXYOS_EVENT_DISPLAY_REMOVED),
        (
            "OXYOS_EVENT_DISPLAY_CONFIRMATION",
            OXYOS_EVENT_DISPLAY_CONFIRMATION,
        ),
    ] {
        assert_eq!(define(name), i64::from(value), "{name}");
    }
    assert_eq!(
        define("OXYOS_DISPLAY_PRIMARY"),
        i64::from(OXYOS_DISPLAY_PRIMARY)
    );
    assert_eq!(
        define("OXYOS_DISPLAY_BUILTIN"),
        i64::from(OXYOS_DISPLAY_BUILTIN)
    );
    // The event kinds are the protocol's opcodes.
    for (event, kind) in [
        (
            Event::DisplayRemoved { output: 2 },
            OXYOS_EVENT_DISPLAY_REMOVED,
        ),
        (
            Event::DisplayConfirmation {
                output: 2,
                seconds: 15,
            },
            OXYOS_EVENT_DISPLAY_CONFIRMATION,
        ),
    ] {
        assert_eq!(u32::from(event.opcode()), kind);
    }
    for name in [
        "oxyos_client_set_display_mode",
        "oxyos_client_set_display_scale",
        "oxyos_client_set_display_origin",
        "oxyos_client_set_primary_display",
        "oxyos_client_confirm_displays",
    ] {
        assert!(HEADER.contains(&format!(" {name}(")), "{name} is declared");
    }
}

#[test]
fn display_events_convert_field_by_field() {
    let name = convert(&Event::DisplayName {
        output: 2,
        index: 1,
        last: true,
        text: PreeditText::new("U2723QE").unwrap(),
    })
    .unwrap();
    assert_eq!(
        (name.kind, name.code, name.detail, name.flag),
        (OXYOS_EVENT_DISPLAY_NAME, 2, 1, 1)
    );
    assert_eq!(&name.text[..8], b"U2723QE\0");
    let mode = convert(&Event::DisplayMode {
        output: 2,
        index: 3,
        width: 2560,
        height: 1440,
        refresh_mhz: 59_951,
        preferred: false,
    })
    .unwrap();
    assert_eq!(
        (
            mode.kind,
            mode.code,
            mode.detail,
            mode.width,
            mode.height,
            mode.x,
            mode.flag
        ),
        (OXYOS_EVENT_DISPLAY_MODE, 2, 3, 2560, 1440, 59_951, 0)
    );
    let scale = convert(&Event::DisplayScale {
        output: 2,
        index: 2,
        scale_120: 180,
        recommended: true,
    })
    .unwrap();
    assert_eq!(
        (
            scale.kind,
            scale.code,
            scale.detail,
            scale.scale_120,
            scale.flag
        ),
        (OXYOS_EVENT_DISPLAY_SCALE, 2, 2, 180, 1)
    );
    let display = convert(&Event::Display {
        output: 2,
        rect: Rect::new(-2560, 480, 2560, 1440),
        scale_120: 180,
        mode: 0,
        width_mm: 597,
        height_mm: 336,
        primary: false,
        builtin: true,
    })
    .unwrap();
    assert_eq!(
        (
            display.kind,
            display.code,
            (display.x, display.y, display.width, display.height),
            display.scale_120,
            display.detail,
            (display.surface, display.serial),
            display.flag
        ),
        (
            OXYOS_EVENT_DISPLAY,
            2,
            (-2560, 480, 2560, 1440),
            180,
            0,
            (597, 336),
            OXYOS_DISPLAY_BUILTIN
        )
    );
    let trial = convert(&Event::DisplayConfirmation {
        output: 2,
        seconds: 15,
    })
    .unwrap();
    assert_eq!(
        (trial.kind, trial.code, trial.detail),
        (OXYOS_EVENT_DISPLAY_CONFIRMATION, 2, 15)
    );
}

#[test]
fn settings_changes_the_displays_through_the_c_abi() {
    let directory =
        std::env::temp_dir().join(format!("oxyos-client-ffi-displays-{}", std::process::id()));
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
            let request = server.recv_request().unwrap().0;
            let done = matches!(request, Request::ConfirmDisplays { .. });
            requests.push(request);
            if done {
                break;
            }
        }
        requests
    });
    let path = CString::new(socket.to_str().unwrap()).unwrap();
    // SAFETY: a NUL-terminated path; the client is closed below.
    unsafe {
        let client = oxyos_client_connect(path.as_ptr());
        assert!(!client.is_null());
        assert_eq!(
            oxyos_client_set_display_mode(client, 1, 2, 2560, 1440, 59_951),
            0
        );
        assert_eq!(oxyos_client_set_display_scale(client, 2, 2, 150), 0);
        assert_eq!(oxyos_client_set_display_origin(client, 3, 2, -2560, 480), 0);
        assert_eq!(oxyos_client_set_primary_display(client, 4, 2), 0);
        assert_eq!(oxyos_client_confirm_displays(client, 5, true), 0);
        assert_eq!(
            oxyos_client_set_display_scale(std::ptr::null_mut(), 6, 2, 150),
            OXYOS_ERROR
        );
        let requests = compositor.join().unwrap();
        assert_eq!(
            requests,
            [
                Request::SetDisplayMode {
                    serial: 1,
                    output: 2,
                    width: 2560,
                    height: 1440,
                    refresh_mhz: 59_951,
                },
                Request::SetDisplayScale {
                    serial: 2,
                    output: 2,
                    scale_120: 150,
                },
                Request::SetDisplayOrigin {
                    serial: 3,
                    output: 2,
                    x: -2560,
                    y: 480,
                },
                Request::SetPrimaryDisplay {
                    serial: 4,
                    output: 2,
                },
                Request::ConfirmDisplays {
                    serial: 5,
                    keep: true,
                },
            ]
        );
        oxyos_client_close(client);
    }
    std::fs::remove_dir_all(directory).unwrap();
}
