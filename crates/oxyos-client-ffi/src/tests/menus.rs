//! The menu bar's C ABI (ADR 0021): the constants and the entry's layout against the header,
//! events field by field, and menus and choices sent through a socket.

use super::*;
use oxyos_protocol::{MenuField, NO_MENU_PARENT};

#[test]
fn the_header_matches_the_menu_bar_abi() {
    for (name, value) in [
        ("OXYOS_EVENT_MENU_BAR", i64::from(OXYOS_EVENT_MENU_BAR)),
        (
            "OXYOS_EVENT_MENU_BAR_ITEM",
            OXYOS_EVENT_MENU_BAR_ITEM.into(),
        ),
        (
            "OXYOS_EVENT_MENU_BAR_TEXT",
            OXYOS_EVENT_MENU_BAR_TEXT.into(),
        ),
        (
            "OXYOS_EVENT_MENU_BAR_COMMITTED",
            OXYOS_EVENT_MENU_BAR_COMMITTED.into(),
        ),
        (
            "OXYOS_EVENT_MENU_ITEM_ACTIVATED",
            OXYOS_EVENT_MENU_ITEM_ACTIVATED.into(),
        ),
        ("OXYOS_MENU_MENU", OXYOS_MENU_MENU.into()),
        ("OXYOS_MENU_ITEM", OXYOS_MENU_ITEM.into()),
        ("OXYOS_MENU_SUBMENU", OXYOS_MENU_SUBMENU.into()),
        ("OXYOS_MENU_SEPARATOR", OXYOS_MENU_SEPARATOR.into()),
        ("OXYOS_MENU_DISABLED", OXYOS_MENU_DISABLED.into()),
        ("OXYOS_MENU_CHECKABLE", OXYOS_MENU_CHECKABLE.into()),
        ("OXYOS_MENU_CHECKED", OXYOS_MENU_CHECKED.into()),
        ("OXYOS_MENU_LABEL", OXYOS_MENU_LABEL.into()),
        ("OXYOS_MENU_SHORTCUT", OXYOS_MENU_SHORTCUT.into()),
        ("OXYOS_NO_MENU_PARENT", OXYOS_NO_MENU_PARENT.into()),
    ] {
        assert_eq!(define(name), value, "{name}");
    }
    // The values are the protocol's.
    assert_eq!(
        [
            MenuKind::Menu as u8,
            MenuKind::Item as u8,
            MenuKind::Submenu as u8,
            MenuKind::Separator as u8
        ],
        [
            OXYOS_MENU_MENU,
            OXYOS_MENU_ITEM,
            OXYOS_MENU_SUBMENU,
            OXYOS_MENU_SEPARATOR
        ]
    );
    assert_eq!(
        [
            MenuFlags::DISABLED,
            MenuFlags::CHECKABLE,
            MenuFlags::CHECKED
        ],
        [
            OXYOS_MENU_DISABLED,
            OXYOS_MENU_CHECKABLE,
            OXYOS_MENU_CHECKED
        ]
    );
    assert_eq!(
        [MenuField::Label as u32, MenuField::Shortcut as u32],
        [OXYOS_MENU_LABEL, OXYOS_MENU_SHORTCUT]
    );
    assert_eq!(NO_MENU_PARENT, OXYOS_NO_MENU_PARENT);
    let opcodes = [
        Event::MenuBar {
            window: 0,
            serial: 0,
            items: 0,
        }
        .opcode(),
        Event::MenuBarCommitted { window: 0 }.opcode(),
        Event::MenuItemActivated {
            surface: 1,
            serial: 1,
            menus: 1,
            index: 0,
        }
        .opcode(),
    ];
    assert_eq!(
        opcodes.map(u32::from),
        [
            OXYOS_EVENT_MENU_BAR,
            OXYOS_EVENT_MENU_BAR_COMMITTED,
            OXYOS_EVENT_MENU_ITEM_ACTIVATED
        ]
    );
    let item = fields("OxyOSMenuItem");
    assert_eq!(
        names(&item),
        ["parent", "kind", "flags", "label", "shortcut"]
    );
    let rust = [
        offset_of!(OxyOSMenuItem, parent),
        offset_of!(OxyOSMenuItem, kind),
        offset_of!(OxyOSMenuItem, flags),
        offset_of!(OxyOSMenuItem, label),
        offset_of!(OxyOSMenuItem, shortcut),
    ];
    assert_eq!(
        c_offsets(&item),
        (rust.to_vec(), size_of::<OxyOSMenuItem>())
    );
    for name in ["oxyos_client_set_menus", "oxyos_client_activate_menu_item"] {
        assert!(HEADER.contains(&format!(" {name}(")), "{name} is declared");
    }
}

#[test]
fn menu_bar_events_convert_field_by_field() {
    let bar = convert(&Event::MenuBar {
        window: 7,
        serial: 3,
        items: 12,
    })
    .unwrap();
    assert_eq!(
        (bar.kind, bar.code, bar.detail, bar.width),
        (OXYOS_EVENT_MENU_BAR, 7, 3, 12)
    );
    let item = convert(&Event::MenuBarItem {
        window: 7,
        index: 4,
        parent: 0,
        kind: MenuKind::Item,
        flags: MenuFlags::new(MenuFlags::CHECKABLE | MenuFlags::CHECKED).unwrap(),
    })
    .unwrap();
    assert_eq!(
        (
            item.kind,
            item.code,
            item.detail,
            item.width,
            item.height,
            item.flag
        ),
        (
            OXYOS_EVENT_MENU_BAR_ITEM,
            7,
            4,
            0,
            u32::from(OXYOS_MENU_ITEM),
            OXYOS_MENU_CHECKABLE | OXYOS_MENU_CHECKED
        )
    );
    let text = convert(&Event::MenuBarText {
        window: 7,
        index: 4,
        field: MenuField::Shortcut,
        text: TextChunk::new("Mod+N").unwrap(),
    })
    .unwrap();
    assert_eq!(
        (text.kind, text.code, text.detail, text.height),
        (OXYOS_EVENT_MENU_BAR_TEXT, 7, 4, OXYOS_MENU_SHORTCUT)
    );
    assert_eq!(&text.text[..6], b"Mod+N\0");
    let committed = convert(&Event::MenuBarCommitted { window: 7 }).unwrap();
    assert_eq!(
        (committed.kind, committed.code),
        (OXYOS_EVENT_MENU_BAR_COMMITTED, 7)
    );
    let chosen = convert(&Event::MenuItemActivated {
        surface: 1,
        serial: 40,
        menus: 3,
        index: 4,
    })
    .unwrap();
    assert_eq!(
        (
            chosen.kind,
            chosen.surface,
            chosen.serial,
            chosen.code,
            chosen.detail
        ),
        (OXYOS_EVENT_MENU_ITEM_ACTIVATED, 1, 40, 3, 4)
    );
}

#[test]
fn menus_and_choices_through_the_c_abi() {
    let directory =
        std::env::temp_dir().join(format!("oxyos-client-ffi-menus-{}", std::process::id()));
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
            let done = matches!(request, Request::ActivateMenuItem { .. });
            requests.push(request);
            if done {
                break;
            }
        }
        server
            .send_event(
                &Event::MenuItemActivated {
                    surface: 1,
                    serial: 40,
                    menus: 2,
                    index: 1,
                },
                None,
            )
            .unwrap();
        requests
    });
    let path = CString::new(socket.to_str().unwrap()).unwrap();
    let file = CString::new("Archivo").unwrap();
    let new = CString::new("Nuevo").unwrap();
    let chord = CString::new("Mod+N").unwrap();
    // A label past the limit is cut at a character; a chord past it is left out.
    let long = CString::new("é".repeat(70)).unwrap();
    let long_chord = CString::new("K".repeat(33)).unwrap();
    let entry = |parent, kind, flags, label: &CString, shortcut: *const c_char| OxyOSMenuItem {
        parent,
        kind,
        flags,
        label: label.as_ptr(),
        shortcut,
    };
    let items = [
        entry(
            OXYOS_NO_MENU_PARENT,
            OXYOS_MENU_MENU,
            0,
            &file,
            std::ptr::null(),
        ),
        entry(0, OXYOS_MENU_ITEM, 0, &new, chord.as_ptr()),
        OxyOSMenuItem {
            label: std::ptr::null(),
            ..entry(0, OXYOS_MENU_SEPARATOR, 0, &file, std::ptr::null())
        },
        entry(
            0,
            OXYOS_MENU_ITEM,
            OXYOS_MENU_CHECKABLE | OXYOS_MENU_CHECKED,
            &long,
            long_chord.as_ptr(),
        ),
    ];
    // SAFETY: the client pointer comes from oxyos_client_connect and is closed once; the entries
    // and their strings outlive the calls.
    unsafe {
        let client = oxyos_client_connect(path.as_ptr());
        assert!(!client.is_null());
        // An unknown kind or flags send nothing.
        let unknown = [entry(OXYOS_NO_MENU_PARENT, 9, 0, &file, std::ptr::null())];
        assert_eq!(
            oxyos_client_set_menus(client, 1, 1, unknown.as_ptr(), 1),
            OXYOS_ERROR
        );
        let flags = [entry(
            OXYOS_NO_MENU_PARENT,
            OXYOS_MENU_MENU,
            0x80,
            &file,
            std::ptr::null(),
        )];
        assert_eq!(
            oxyos_client_set_menus(client, 1, 1, flags.as_ptr(), 1),
            OXYOS_ERROR
        );
        assert_eq!(
            oxyos_client_set_menus(client, 1, 1, std::ptr::null(), 2),
            OXYOS_ERROR
        );
        assert_eq!(
            oxyos_client_set_menus(client, 1, 2, items.as_ptr(), items.len()),
            0
        );
        assert_eq!(oxyos_client_activate_menu_item(client, 7, 2, 1), 0);
        let requests = compositor.join().unwrap();
        let mut event = OxyOSEvent::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while oxyos_client_poll(client, &mut event) != OXYOS_POLL_EVENT {
            assert!(std::time::Instant::now() < deadline, "the choice arrives");
        }
        assert_eq!(
            (
                event.kind,
                event.surface,
                event.serial,
                event.code,
                event.detail
            ),
            (OXYOS_EVENT_MENU_ITEM_ACTIVATED, 1, 40, 2, 1)
        );
        oxyos_client_close(client);
        assert_eq!(
            requests.first(),
            Some(&Request::MenusBegin {
                surface: 1,
                serial: 2,
                items: 4
            })
        );
        assert_eq!(
            requests.last(),
            Some(&Request::ActivateMenuItem {
                window: 7,
                serial: 2,
                index: 1
            })
        );
        assert_eq!(
            requests[requests.len() - 2],
            Request::MenusCommit { surface: 1 }
        );
        // Each entry's texts, as sent.
        let mut texts: std::collections::BTreeMap<(u32, u8), String> = Default::default();
        let mut kinds = Vec::new();
        for request in &requests {
            match request {
                Request::MenuItem {
                    index,
                    parent,
                    kind,
                    flags,
                    ..
                } => kinds.push((*index, *parent, *kind as u8, flags.bits())),
                Request::MenuText {
                    index, field, text, ..
                } => texts
                    .entry((*index, *field as u8))
                    .or_default()
                    .push_str(text.as_str()),
                _ => {}
            }
        }
        assert_eq!(
            kinds,
            [
                (0, NO_MENU_PARENT, OXYOS_MENU_MENU, 0),
                (1, 0, OXYOS_MENU_ITEM, 0),
                (2, 0, OXYOS_MENU_SEPARATOR, 0),
                (
                    3,
                    0,
                    OXYOS_MENU_ITEM,
                    OXYOS_MENU_CHECKABLE | OXYOS_MENU_CHECKED
                ),
            ]
        );
        let label = MenuField::Label as u8;
        let shortcut = MenuField::Shortcut as u8;
        assert_eq!(texts[&(0, label)], "Archivo");
        assert_eq!(texts[&(1, label)], "Nuevo");
        assert_eq!(texts[&(1, shortcut)], "Mod+N");
        assert_eq!(texts[&(3, label)], "é".repeat(64));
        assert!(!texts.contains_key(&(2, label)));
        assert!(!texts.contains_key(&(3, shortcut)));
    }
    std::fs::remove_dir_all(directory).unwrap();
}
