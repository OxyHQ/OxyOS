//! The menu bar (ADR 0021): windows' menus, the shell told the active window's, and its choices.

use super::*;
use crate::scene::MenuEntry;
use oxyos_protocol::{
    MenuField, MenuFlags, MenuKind, MAX_MENU_DEPTH, MAX_MENU_ITEMS, MAX_MENU_LABEL_BYTES,
    MAX_MENU_SHORTCUT_BYTES, NO_MENU_PARENT,
};

/// An entry: (parent, kind, flags, label, shortcut).
type Entry<'a> = (u32, MenuKind, u8, &'a str, &'a str);

const NONE: u32 = NO_MENU_PARENT;
const CHECK: u8 = MenuFlags::CHECKABLE | MenuFlags::CHECKED;

/// File (New, Open…, a separator, Wrap lines checked, Recent ▸ Disabled one) and Edit (Undo
/// disabled, Copy). Indices: 0 File, 1 New, 2 Open, 3 separator, 4 Wrap, 5 Recent, 6 Report,
/// 7 Edit, 8 Undo, 9 Copy.
fn sample() -> Vec<Entry<'static>> {
    vec![
        (NONE, MenuKind::Menu, 0, "Archivo", ""),
        (0, MenuKind::Item, 0, "Nuevo", "Mod+N"),
        (0, MenuKind::Item, 0, "Abrir…", "Mod+O"),
        (0, MenuKind::Separator, 0, "", ""),
        (0, MenuKind::Item, CHECK, "Ajustar líneas", ""),
        (0, MenuKind::Submenu, MenuFlags::DISABLED, "Recientes", ""),
        (5, MenuKind::Item, 0, "informe.txt", ""),
        (NONE, MenuKind::Menu, 0, "Edición", ""),
        (7, MenuKind::Item, MenuFlags::DISABLED, "Deshacer", "Mod+Z"),
        (7, MenuKind::Item, 0, "Copiar", "Mod+C"),
    ]
}

/// The requests declaring `entries` as the commit `serial` of `surface`.
fn requests(surface: u32, serial: u32, entries: &[Entry<'_>]) -> Vec<Request> {
    let mut out = vec![Request::MenusBegin {
        surface,
        serial,
        items: entries.len() as u32,
    }];
    for (index, (parent, kind, flags, label, shortcut)) in (0..).zip(entries) {
        out.push(Request::MenuItem {
            surface,
            index,
            parent: *parent,
            kind: *kind,
            flags: MenuFlags::new(*flags).unwrap(),
        });
        for (field, text) in [(MenuField::Label, label), (MenuField::Shortcut, shortcut)] {
            for text in TextChunk::split(text) {
                out.push(Request::MenuText {
                    surface,
                    index,
                    field,
                    text,
                });
            }
        }
    }
    out.push(Request::MenusCommit { surface });
    out
}

/// Declares the menus; the first error, if any.
fn declare(
    h: &mut Harness,
    window: SurfaceKey,
    serial: u32,
    entries: &[Entry<'_>],
) -> Result<(), ErrorCode> {
    for request in requests(window.id, serial, entries) {
        h.request(window.client, request)?;
    }
    Ok(())
}

/// What a shell was told of the bar, each time it was told: the window, the serial and its
/// entries.
type Bar = (u32, u32, Vec<MenuEntry>);
fn bars(events: &[(ClientId, Event)], client: ClientId) -> Vec<Bar> {
    let mut told = Vec::new();
    let mut receiving: Option<(u32, u32, u32, Vec<MenuEntry>)> = None;
    for event in to(events, client) {
        match event {
            Event::MenuBar {
                window,
                serial,
                items,
            } => receiving = Some((window, serial, items, Vec::new())),
            Event::MenuBarItem {
                window,
                index,
                parent,
                kind,
                flags,
            } => {
                let (at, _, _, entries) = receiving.as_mut().expect("begun");
                assert_eq!((*at, index as usize), (window, entries.len()));
                entries.push(MenuEntry {
                    parent,
                    kind,
                    flags,
                    label: String::new(),
                    shortcut: String::new(),
                });
            }
            Event::MenuBarText {
                window,
                index,
                field,
                text,
            } => {
                let (at, _, _, entries) = receiving.as_mut().expect("begun");
                assert_eq!(*at, window);
                let entry = &mut entries[index as usize];
                match field {
                    MenuField::Label => entry.label.push_str(text.as_str()),
                    MenuField::Shortcut => entry.shortcut.push_str(text.as_str()),
                }
            }
            Event::MenuBarCommitted { window } => {
                let (at, serial, items, entries) = receiving.take().expect("begun");
                assert_eq!((at, items as usize), (window, entries.len()));
                told.push((window, serial, entries));
            }
            _ => {}
        }
    }
    assert!(receiving.is_none(), "a bar begun and never committed");
    told
}

fn entries(declared: &[Entry<'_>]) -> Vec<MenuEntry> {
    declared
        .iter()
        .map(|(parent, kind, flags, label, shortcut)| MenuEntry {
            parent: *parent,
            kind: *kind,
            flags: MenuFlags::new(*flags).unwrap(),
            label: label.to_string(),
            shortcut: shortcut.to_string(),
        })
        .collect()
}

fn choose(h: &mut Harness, window: u32, serial: u32, index: u32) -> Vec<(ClientId, Event)> {
    h.clear_log();
    h.request(
        SHELL,
        Request::ActivateMenuItem {
            window,
            serial,
            index,
        },
    )
    .unwrap();
    logged(h)
}

/// What a choice delivered: (client, surface, menus, index).
fn chosen(events: &[(ClientId, Event)]) -> Vec<(ClientId, u32, u32, u32)> {
    events
        .iter()
        .filter_map(|(client, event)| match event {
            Event::MenuItemActivated {
                surface,
                menus,
                index,
                ..
            } => Some((*client, *surface, *menus, *index)),
            _ => None,
        })
        .collect()
}

#[test]
fn the_shell_is_told_the_active_windows_menus_as_activation_follows() {
    let mut h = Harness::new();
    // Nothing is active: the shell is told so at once.
    assert_eq!(bars(&logged(&mut h), SHELL), [(0, 0, vec![])]);
    let inbox = h.toplevel(APP, 1, 400, 300);
    declare(&mut h, inbox, 1, &sample()).unwrap();
    let a = h.scene.window_number(inbox).unwrap();
    let events = logged(&mut h);
    // Told the window as it became active, then its menus: the last is what it shows.
    assert_eq!(
        bars(&events, SHELL).last(),
        Some(&(a, 1, entries(&sample())))
    );
    assert_eq!(h.scene.menus(inbox).unwrap().serial, 1);
    // Another window becomes active: its menus (none).
    let other = h.toplevel(OTHER, 1, 300, 200);
    let b = h.scene.window_number(other).unwrap();
    assert_eq!(bars(&logged(&mut h), SHELL), [(b, 0, vec![])]);
    // Back to the first: its menus again, told whole.
    h.activate(inbox);
    assert_eq!(bars(&logged(&mut h), SHELL), [(a, 1, entries(&sample()))]);
    // Nothing changed: nothing told.
    h.frame();
    assert_eq!(bars(&logged(&mut h), SHELL), []);
    // A recommit replaces them.
    let shorter = &sample()[..2];
    declare(&mut h, inbox, 2, shorter).unwrap();
    assert_eq!(bars(&logged(&mut h), SHELL), [(a, 2, entries(shorter))]);
    // The shell's panel taking focus (its top bar) leaves them: the window stays active.
    h.request(
        SHELL,
        Request::CreateShellSurface {
            surface: 1,
            kind: ShellKind::Panel,
            edge: PanelEdge::Top,
            output: 1,
            size: Size::new(1, 28),
            exclusive: 28,
        },
    )
    .unwrap();
    h.present(key(SHELL, 1));
    h.clear_log();
    h.click(10, 10);
    assert_eq!(h.scene.focus(), Some(key(SHELL, 1)));
    assert_eq!(bars(&logged(&mut h), SHELL), []);
    // Locked, no window is active; unlocked, the bar comes back.
    h.scene.lock();
    assert_eq!(bars(&logged(&mut h), SHELL), [(0, 0, vec![])]);
    h.scene.unlock();
    let back = bars(&logged(&mut h), SHELL);
    assert_eq!(back.last(), Some(&(a, 2, entries(shorter))));
    // Minimized, the next window is active: its bar.
    h.scene.minimize(inbox);
    assert_eq!(bars(&logged(&mut h), SHELL), [(b, 0, vec![])]);
    h.scene.restore(inbox);
    assert_eq!(bars(&logged(&mut h), SHELL), [(a, 2, entries(shorter))]);
    // A shell that connects later is told the bar at once.
    h.scene
        .connect(
            7,
            Grants {
                shell: true,
                ..Grants::default()
            },
        )
        .unwrap();
    assert_eq!(bars(&logged(&mut h), 7), [(a, 2, entries(shorter))]);
}

#[test]
fn applications_hear_nothing_of_any_windows_menus() {
    let mut h = Harness::new();
    let inbox = h.toplevel(APP, 1, 400, 300);
    declare(&mut h, inbox, 1, &sample()).unwrap();
    let other = h.toplevel(OTHER, 1, 300, 200);
    declare(&mut h, other, 1, &sample()[..2]).unwrap();
    h.activate(inbox);
    h.activate(other);
    let a = h.scene.window_number(inbox).unwrap();
    choose(&mut h, a, 1, 1);
    for client in [APP, OTHER, LOCKER] {
        assert!(
            h.events_for(client).iter().all(|event| !is_menu_bar(event)),
            "client {client} was told a bar"
        );
    }
    // A client without the shell grant cannot choose, and is refused for trying.
    assert_eq!(
        h.request(
            OTHER,
            Request::ActivateMenuItem {
                window: a,
                serial: 1,
                index: 1,
            },
        ),
        Err(ErrorCode::Unauthorized)
    );
}

#[test]
fn a_choice_reaches_the_window_only_for_a_command_it_declares_now() {
    let mut h = Harness::new();
    let inbox = h.toplevel(APP, 1, 400, 300);
    declare(&mut h, inbox, 1, &sample()).unwrap();
    let a = h.scene.window_number(inbox).unwrap();
    // New: to the window's client alone, once, with a serial that is the person's input.
    let events = choose(&mut h, a, 1, 1);
    assert_eq!(chosen(&events), [(APP, 1, 1, 1)]);
    let Some(Event::MenuItemActivated { serial, .. }) = to(&events, APP).first().copied() else {
        panic!("delivered");
    };
    assert!(h.scene.recent_input(APP, serial));
    // The check item too: the application toggles it and commits again.
    assert_eq!(chosen(&choose(&mut h, a, 1, 4)), [(APP, 1, 1, 4)]);
    // Refused, with nothing sent: a menu, a separator, a disabled command, a command in a
    // disabled submenu, an index past the end, another serial, another window's number.
    for (window, serial, index) in [
        (a, 1, 0),
        (a, 1, 3),
        (a, 1, 8),
        (a, 1, 6),
        (a, 1, 5),
        (a, 1, 10),
        (a, 2, 1),
        (a, 0, 1),
        (a + 1, 1, 1),
        (0, 1, 1),
    ] {
        assert_eq!(
            chosen(&choose(&mut h, window, serial, index)),
            [],
            "{window} {serial} {index}"
        );
    }
    // After a recommit, a choice from the menus before it is stale: refused, never another
    // entry that took its index.
    let reordered = vec![
        (NONE, MenuKind::Menu, 0, "Archivo", ""),
        (0, MenuKind::Item, 0, "Cerrar", "Mod+W"),
        (0, MenuKind::Item, 0, "Nuevo", "Mod+N"),
    ];
    declare(&mut h, inbox, 5, &reordered).unwrap();
    assert_eq!(chosen(&choose(&mut h, a, 1, 1)), []);
    assert_eq!(chosen(&choose(&mut h, a, 5, 2)), [(APP, 1, 5, 2)]);
    // Serials only rise.
    assert_eq!(
        declare(&mut h, inbox, 5, &reordered),
        Err(ErrorCode::InvalidMessage)
    );
    // A window that is not active is not chosen from, whatever its menus.
    let mut h = Harness::new();
    let inbox = h.toplevel(APP, 1, 400, 300);
    declare(&mut h, inbox, 1, &sample()).unwrap();
    let a = h.scene.window_number(inbox).unwrap();
    h.toplevel(OTHER, 1, 300, 200);
    assert_eq!(chosen(&choose(&mut h, a, 1, 1)), []);
    // Nor while locked.
    h.activate(inbox);
    h.scene.lock();
    assert_eq!(chosen(&choose(&mut h, a, 1, 1)), []);
    h.scene.unlock();
    assert_eq!(chosen(&choose(&mut h, a, 1, 1)), [(APP, 1, 1, 1)]);
}

#[test]
fn a_window_that_goes_takes_its_menus_off_the_bar() {
    let mut h = Harness::new();
    let other = h.toplevel(OTHER, 1, 300, 200);
    let inbox = h.toplevel(APP, 1, 400, 300);
    declare(&mut h, inbox, 1, &sample()).unwrap();
    let a = h.scene.window_number(inbox).unwrap();
    let b = h.scene.window_number(other).unwrap();
    h.clear_log();
    h.request(APP, Request::Destroy { surface: 1 }).unwrap();
    assert_eq!(bars(&logged(&mut h), SHELL), [(b, 0, vec![])]);
    assert!(h.scene.menus(inbox).is_none());
    assert_eq!(chosen(&choose(&mut h, a, 1, 1)), []);
    // The last window's client going: nothing is active, the bar is the shell's own.
    h.scene.disconnect(OTHER);
    assert_eq!(bars(&logged(&mut h), SHELL), [(0, 0, vec![])]);
    // Menus committed empty are none.
    let mut h = Harness::new();
    let inbox = h.toplevel(APP, 1, 400, 300);
    declare(&mut h, inbox, 1, &sample()).unwrap();
    h.clear_log();
    declare(&mut h, inbox, 2, &[]).unwrap();
    let a = h.scene.window_number(inbox).unwrap();
    assert_eq!(bars(&logged(&mut h), SHELL), [(a, 0, vec![])]);
}

#[test]
fn menus_within_their_limits_are_taken_and_malformed_ones_are_protocol_errors() {
    let long = "a".repeat(MAX_MENU_LABEL_BYTES);
    let chord = "K".repeat(MAX_MENU_SHORTCUT_BYTES);
    // At every limit: entries, depth, a label's and a shortcut's bytes.
    let mut most = vec![(NONE, MenuKind::Menu, 0, long.as_str(), "")];
    for depth in 2..MAX_MENU_DEPTH {
        most.push((depth - 2, MenuKind::Submenu, 0, "Más", ""));
    }
    let deepest = most.len() as u32 - 1;
    while most.len() < MAX_MENU_ITEMS as usize {
        most.push((deepest, MenuKind::Item, 0, "x", chord.as_str()));
    }
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    declare(&mut h, window, 1, &most).unwrap();
    assert_eq!(h.scene.menus(window).unwrap().entries.len(), most.len());

    let file = (NONE, MenuKind::Menu, 0, "Archivo", "");
    let new = (0, MenuKind::Item, 0, "Nuevo", "");
    let item = |parent, kind, flags| -> Entry<'static> { (parent, kind, flags, "x", "") };
    let mut too_deep = vec![file];
    // A menu, then submenus each in the last, one deeper than the limit.
    for depth in 1..=MAX_MENU_DEPTH {
        too_deep.push((depth - 1, MenuKind::Submenu, 0, "Más", ""));
    }
    let malformed: Vec<(Vec<Entry<'_>>, ErrorCode)> = vec![
        // A root that is not a menu, a menu that is not a root.
        (vec![new], ErrorCode::InvalidMessage),
        (
            vec![file, item(0, MenuKind::Menu, 0)],
            ErrorCode::InvalidMessage,
        ),
        // A parent that is not earlier, or not a menu or submenu.
        (
            vec![file, item(1, MenuKind::Item, 0)],
            ErrorCode::InvalidMessage,
        ),
        (
            vec![file, new, item(1, MenuKind::Item, 0)],
            ErrorCode::InvalidMessage,
        ),
        // Flags a kind cannot have, a check without one.
        (
            vec![file, item(0, MenuKind::Item, MenuFlags::CHECKED)],
            ErrorCode::InvalidMessage,
        ),
        (
            vec![(NONE, MenuKind::Menu, MenuFlags::DISABLED, "x", "")],
            ErrorCode::InvalidMessage,
        ),
        (
            vec![file, item(0, MenuKind::Submenu, MenuFlags::CHECKABLE)],
            ErrorCode::InvalidMessage,
        ),
        (
            vec![file, (0, MenuKind::Separator, MenuFlags::DISABLED, "", "")],
            ErrorCode::InvalidMessage,
        ),
        // Text on a separator, a shortcut on anything but a command, an entry with no label.
        (
            vec![file, (0, MenuKind::Separator, 0, "—", "")],
            ErrorCode::InvalidMessage,
        ),
        (
            vec![(NONE, MenuKind::Menu, 0, "x", "Mod+N")],
            ErrorCode::InvalidMessage,
        ),
        (
            vec![file, (0, MenuKind::Item, 0, "", "")],
            ErrorCode::InvalidMessage,
        ),
        // Too deep.
        (too_deep, ErrorCode::QuotaExceeded),
    ];
    for (entries, error) in malformed {
        let mut h = Harness::new();
        let window = h.toplevel(APP, 1, 400, 300);
        assert_eq!(
            declare(&mut h, window, 1, &entries),
            Err(error),
            "{entries:?}"
        );
    }

    // Bounds and order, request by request.
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let begin = |items| Request::MenusBegin {
        surface: 1,
        serial: 1,
        items,
    };
    assert_eq!(
        h.request(APP, begin(MAX_MENU_ITEMS + 1)),
        Err(ErrorCode::QuotaExceeded)
    );
    // Nothing begun.
    assert_eq!(
        h.request(APP, Request::MenusCommit { surface: 1 }),
        Err(ErrorCode::InvalidOrder)
    );
    let root = |index| Request::MenuItem {
        surface: 1,
        index,
        parent: NONE,
        kind: MenuKind::Menu,
        flags: MenuFlags::default(),
    };
    let label = |index, text: &str| Request::MenuText {
        surface: 1,
        index,
        field: MenuField::Label,
        text: TextChunk::new(text).unwrap(),
    };
    for (requests, error) in [
        // Out of order, more than announced.
        (vec![begin(2), root(1)], ErrorCode::InvalidMessage),
        (vec![begin(1), root(0), root(1)], ErrorCode::InvalidMessage),
        // Text for an entry not sent, a label past its limit.
        (vec![begin(1), label(0, "x")], ErrorCode::InvalidOrder),
        (
            [
                vec![begin(1), root(0)],
                vec![label(0, &"a".repeat(32)); 4],
                vec![label(0, "b")],
            ]
            .concat(),
            ErrorCode::QuotaExceeded,
        ),
        // Committed before every entry came.
        (
            vec![
                begin(2),
                root(0),
                label(0, "x"),
                Request::MenusCommit { surface: 1 },
            ],
            ErrorCode::InvalidOrder,
        ),
    ] {
        let mut h = Harness::new();
        h.toplevel(APP, 1, 400, 300);
        let result = requests.into_iter().try_for_each(|r| h.request(APP, r));
        assert_eq!(result, Err(error));
    }
    // Only a toplevel has menus; a surface that is not there has none.
    let dialog = Request::CreateDialog {
        surface: 2,
        parent: 1,
        size: Size::new(100, 100),
    };
    h.request(APP, dialog).unwrap();
    assert_eq!(
        declare(&mut h, key(APP, 2), 1, &[file]),
        Err(ErrorCode::InvalidRole)
    );
    assert_eq!(
        declare(&mut h, key(APP, 9), 1, &[file]),
        Err(ErrorCode::UnknownSurface)
    );
    declare(&mut h, window, 1, &[file]).unwrap();
}
