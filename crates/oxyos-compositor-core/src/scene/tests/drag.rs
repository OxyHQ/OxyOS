//! Drag and drop between applications (ADR 0035): a drag from a held press, its target the
//! surface under the pointer and no other, the action negotiated, the drop received through the
//! source once per type, and every way a drag ends without one.

use super::*;
use crate::scene::{negotiate_action, DRAG_VERSION, MAX_OPEN_DROPS};
use oxyos_protocol::{DragActions, DragCancelReason, MODIFIER_ALT, MODIFIER_SUPER};

const ALT: u32 = 56;
const URIS: &str = "text/uri-list";
const TEXT: &str = "text/plain;charset=utf-8";

fn mime(name: &str) -> TextChunk {
    TextChunk::new(name).unwrap()
}

/// Three windows side by side: the source's (APP), the target's (OTHER) and a third (SHELL's
/// stand-in for any other application, a toplevel of its own), each 400 x 300.
struct Desk {
    h: Harness,
    source: SurfaceKey,
    target: SurfaceKey,
    third: SurfaceKey,
}

fn desk() -> Desk {
    let mut h = Harness::new();
    let source = h.toplevel(APP, 1, 400, 300);
    let target = h.toplevel(OTHER, 1, 400, 300);
    let third = h.toplevel(LOCKER, 1, 400, 300);
    // Side by side, apart, as the person arranged them.
    h.scene.move_window_to(source, (0, 100));
    h.scene.move_window_to(target, (600, 100));
    h.scene.move_window_to(third, (1200, 100));
    h.frame();
    h.clear_log();
    Desk {
        h,
        source,
        target,
        third,
    }
}

fn center(h: &Harness, surface: SurfaceKey) -> (i32, i32) {
    let rect = h.rect(surface);
    (
        rect.x + rect.width as i32 / 2,
        rect.y + rect.height as i32 / 2,
    )
}

/// The source offers `types` and starts a drag from a press in the middle of its window.
fn start(d: &mut Desk, types: &[&str], actions: DragActions) -> u64 {
    let (x, y) = center(&d.h, d.source);
    let serial = d.h.press(x, y);
    for name in types {
        d.h.request(
            APP,
            Request::DragType {
                drag: 7,
                mime: mime(name),
            },
        )
        .unwrap();
    }
    d.h.request(
        APP,
        Request::StartDrag {
            drag: 7,
            surface: d.source.id,
            icon: 0,
            serial,
            actions,
        },
    )
    .unwrap();
    serial
}

fn drag_events(events: &[Event]) -> Vec<Event> {
    events
        .iter()
        .filter(|event| event.opcode() >= 240)
        .copied()
        .collect()
}

fn entering(events: &[Event]) -> Option<(u32, u64)> {
    events.iter().find_map(|event| match event {
        Event::DragEnter { offer, serial, .. } => Some((*offer, *serial)),
        _ => None,
    })
}

fn accept(h: &mut Harness, client: ClientId, offer: u32, serial: u64, name: &str) {
    h.request(
        client,
        Request::AcceptDrag {
            offer,
            serial,
            mime: PreeditText::new(name).unwrap(),
            actions: DragActions::COPY.union(DragActions::MOVE),
            preferred: DragActions::MOVE,
        },
    )
    .unwrap();
}

#[test]
fn a_drag_reaches_the_window_under_the_pointer_with_its_types_and_no_other() {
    let mut d = desk();
    start(
        &mut d,
        &[URIS, TEXT],
        DragActions::COPY.union(DragActions::MOVE),
    );
    let events = logged(&mut d.h);
    // The press is the drag's: the source is told it was cancelled and the pointer left, then it
    // is the drag's first target, its own window being under the pointer.
    let source = to(&events, APP);
    assert!(source.iter().any(|event| matches!(
        event,
        Event::Pointer {
            kind: PointerKind::Cancel,
            ..
        }
    )));
    assert!(matches!(
        drag_events(&source)[..],
        [
            Event::DragEnter {
                surface: 1,
                types: 2,
                ..
            },
            Event::DragOfferType { .. },
            Event::DragOfferType { .. },
        ]
    ));
    assert!(d.h.scene.dragging());
    // Over the target: the source's window is left, the target entered with the types, in order.
    let (x, y) = center(&d.h, d.target);
    d.h.scene.pointer_motion(x, y);
    d.h.scene.pointer_motion(x + 5, y + 5);
    let events = logged(&mut d.h);
    assert!(matches!(
        drag_events(&to(&events, APP))[..],
        [Event::DragLeave { surface: 1, .. }]
    ));
    let target = drag_events(&to(&events, OTHER));
    let offer = entering(&target).unwrap().0;
    assert_eq!(
        target[1..3],
        [
            Event::DragOfferType {
                offer,
                mime: mime(URIS)
            },
            Event::DragOfferType {
                offer,
                mime: mime(TEXT)
            }
        ]
    );
    assert!(matches!(target[3], Event::DragMotion { surface: 1, .. }));
    // No pointer event reaches anyone during the drag, and the third window, which the pointer
    // never crossed, learns nothing at all.
    assert!(events
        .iter()
        .all(|(_, event)| !matches!(event, Event::Pointer { .. })));
    assert!(to(&events, LOCKER).is_empty());
    let _ = d.third;
}

#[test]
fn only_a_held_press_of_the_source_on_that_window_starts_a_drag() {
    let refused = |events: &[(ClientId, Event)]| {
        to(events, APP).contains(&Event::DragCancelled {
            drag: 7,
            reason: DragCancelReason::Refused,
        })
    };
    // Another client's press.
    let mut d = desk();
    let (x, y) = center(&d.h, d.target);
    let serial = d.h.press(x, y);
    d.h.request(
        APP,
        Request::DragType {
            drag: 7,
            mime: mime(TEXT),
        },
    )
    .unwrap();
    d.h.request(
        APP,
        Request::StartDrag {
            drag: 7,
            surface: d.source.id,
            icon: 0,
            serial,
            actions: DragActions::COPY,
        },
    )
    .unwrap();
    assert!(refused(&logged(&mut d.h)) && !d.h.scene.dragging());
    // A press released already.
    let mut d = desk();
    let (x, y) = center(&d.h, d.source);
    let serial = d.h.press(x, y);
    d.h.scene.pointer_button(BUTTON, false);
    d.h.request(
        APP,
        Request::DragType {
            drag: 7,
            mime: mime(TEXT),
        },
    )
    .unwrap();
    d.h.request(
        APP,
        Request::StartDrag {
            drag: 7,
            surface: d.source.id,
            icon: 0,
            serial,
            actions: DragActions::COPY,
        },
    )
    .unwrap();
    assert!(refused(&logged(&mut d.h)) && !d.h.scene.dragging());
    // No types, or while locked.
    let mut d = desk();
    let (x, y) = center(&d.h, d.source);
    let serial = d.h.press(x, y);
    d.h.request(
        APP,
        Request::StartDrag {
            drag: 7,
            surface: d.source.id,
            icon: 0,
            serial,
            actions: DragActions::COPY,
        },
    )
    .unwrap();
    assert!(refused(&logged(&mut d.h)));
    let mut d = desk();
    let (x, y) = center(&d.h, d.source);
    let serial = d.h.press(x, y);
    d.h.scene.lock();
    d.h.request(
        APP,
        Request::DragType {
            drag: 7,
            mime: mime(TEXT),
        },
    )
    .unwrap();
    d.h.request(
        APP,
        Request::StartDrag {
            drag: 7,
            surface: d.source.id,
            icon: 0,
            serial,
            actions: DragActions::COPY,
        },
    )
    .unwrap();
    assert!(refused(&logged(&mut d.h)) && !d.h.scene.dragging());
}

#[test]
fn a_drag_is_refused_while_a_menu_holds_the_grab() {
    let mut d = desk();
    let window = d.source;
    let rect = d.h.rect(window);
    let menu = d.h.open_menu(
        window,
        2,
        (rect.x + 10, rect.y + 10),
        menu(Rect::new(10, 10, 20, 20), 100, 100),
    );
    let menu_rect = d.h.rect(menu);
    let serial = d.h.press(menu_rect.x + 5, menu_rect.y + 5);
    d.h.request(
        APP,
        Request::DragType {
            drag: 7,
            mime: mime(TEXT),
        },
    )
    .unwrap();
    d.h.request(
        APP,
        Request::StartDrag {
            drag: 7,
            surface: menu.id,
            icon: 0,
            serial,
            actions: DragActions::COPY,
        },
    )
    .unwrap();
    assert!(!d.h.scene.dragging());
}

#[test]
fn the_action_is_chosen_as_a_mac_chooses_it() {
    let all = DragActions::COPY
        .union(DragActions::MOVE)
        .union(DragActions::LINK);
    let copy_move = DragActions::COPY.union(DragActions::MOVE);
    // The target's preferred action, when both allow it.
    assert_eq!(
        negotiate_action(all, copy_move, DragActions::MOVE, true, 0),
        DragActions::MOVE
    );
    // Else the first both allow of copy, move, link.
    assert_eq!(
        negotiate_action(all, copy_move, DragActions::LINK, true, 0),
        DragActions::COPY
    );
    assert_eq!(
        negotiate_action(DragActions::LINK, all, DragActions::NONE, true, 0),
        DragActions::LINK
    );
    // Option forces a copy, Command a move, both a link; none if either side refuses it.
    assert_eq!(
        negotiate_action(all, all, DragActions::MOVE, true, MODIFIER_ALT),
        DragActions::COPY
    );
    assert_eq!(
        negotiate_action(all, all, DragActions::COPY, true, MODIFIER_SUPER),
        DragActions::MOVE
    );
    assert_eq!(
        negotiate_action(
            all,
            all,
            DragActions::COPY,
            true,
            MODIFIER_ALT | MODIFIER_SUPER
        ),
        DragActions::LINK
    );
    assert_eq!(
        negotiate_action(
            all,
            DragActions::MOVE,
            DragActions::MOVE,
            true,
            MODIFIER_ALT
        ),
        DragActions::NONE
    );
    // Nothing both allow, or no type taken: none.
    assert_eq!(
        negotiate_action(
            DragActions::COPY,
            DragActions::MOVE,
            DragActions::MOVE,
            true,
            0
        ),
        DragActions::NONE
    );
    assert_eq!(
        negotiate_action(all, all, DragActions::MOVE, false, 0),
        DragActions::NONE
    );
}

#[test]
fn both_sides_are_told_the_action_and_option_held_turns_a_move_into_a_copy() {
    let mut d = desk();
    start(&mut d, &[URIS], DragActions::COPY.union(DragActions::MOVE));
    let (x, y) = center(&d.h, d.target);
    d.h.scene.pointer_motion(x, y);
    let events = logged(&mut d.h);
    let (offer, serial) = entering(&to(&events, OTHER)).unwrap();
    // An answer citing another entering is ignored.
    accept(&mut d.h, OTHER, offer, serial + 1000, URIS);
    assert!(drag_events(&to(&logged(&mut d.h), OTHER)).is_empty());
    accept(&mut d.h, OTHER, offer, serial, URIS);
    let events = logged(&mut d.h);
    assert_eq!(
        drag_events(&to(&events, OTHER)),
        [Event::DragAction {
            offer,
            action: DragActions::MOVE
        }]
    );
    assert_eq!(
        drag_events(&to(&events, APP)),
        [Event::DragSourceAction {
            drag: 7,
            action: DragActions::MOVE
        }]
    );
    // Option held: a copy, told to both; let go, a move again.
    d.h.scene.key(ALT, true);
    let events = logged(&mut d.h);
    assert!(to(&events, OTHER).contains(&Event::DragAction {
        offer,
        action: DragActions::COPY
    }));
    assert!(to(&events, APP).contains(&Event::DragSourceAction {
        drag: 7,
        action: DragActions::COPY
    }));
    d.h.scene.key(ALT, false);
    assert!(to(&logged(&mut d.h), OTHER).contains(&Event::DragAction {
        offer,
        action: DragActions::MOVE
    }));
    // A type the drag does not offer is no answer: nothing would be dropped.
    accept(&mut d.h, OTHER, offer, serial, "image/png");
    assert!(
        to(&logged(&mut d.h), APP).contains(&Event::DragSourceAction {
            drag: 7,
            action: DragActions::NONE
        })
    );
}

/// Drags `types` from the source onto the target, which accepts `URIS`, and drops: the drop's
/// offer.
fn drop_on_target(d: &mut Desk, types: &[&str]) -> u32 {
    start(d, types, DragActions::COPY.union(DragActions::MOVE));
    let (x, y) = center(&d.h, d.target);
    d.h.scene.pointer_motion(x, y);
    let (offer, serial) = entering(&to(&logged(&mut d.h), OTHER)).unwrap();
    accept(&mut d.h, OTHER, offer, serial, URIS);
    d.h.clear_log();
    d.h.scene.pointer_button(BUTTON, false);
    offer
}

#[test]
fn a_drop_reaches_the_target_that_accepted_and_each_type_is_received_once() {
    let mut d = desk();
    let offer = drop_on_target(&mut d, &[URIS, TEXT]);
    let events = logged(&mut d.h);
    let dropped: Vec<Event> = drag_events(&to(&events, OTHER));
    let Event::DragDrop {
        surface: 1,
        offer: dropped_offer,
        serial,
        action: DragActions::MOVE,
        ..
    } = dropped[0]
    else {
        panic!("{dropped:?}");
    };
    assert_eq!(dropped_offer, offer);
    assert_eq!(
        drag_events(&to(&events, APP)),
        [Event::DragDropped {
            drag: 7,
            action: DragActions::MOVE
        }]
    );
    assert!(!d.h.scene.dragging());
    // The drop is a user input of the target's.
    assert!(d.h.scene.recent_input(OTHER, serial));
    // Another client, a type not offered: refused, the source asked nothing.
    d.h.request(
        LOCKER,
        Request::ReceiveDrop {
            offer,
            mime: mime(URIS),
        },
    )
    .unwrap();
    assert_eq!(d.h.scene.take_transfer(), None);
    d.h.request(
        OTHER,
        Request::ReceiveDrop {
            offer,
            mime: mime("image/png"),
        },
    )
    .unwrap();
    assert_eq!(d.h.scene.take_transfer(), None);
    // The target's: the source is asked for that type, once.
    d.h.request(
        OTHER,
        Request::ReceiveDrop {
            offer,
            mime: mime(URIS),
        },
    )
    .unwrap();
    assert_eq!(d.h.scene.take_transfer(), Some(APP));
    d.h.request(
        OTHER,
        Request::ReceiveDrop {
            offer,
            mime: mime(URIS),
        },
    )
    .unwrap();
    assert_eq!(d.h.scene.take_transfer(), None, "a type once");
    d.h.request(
        OTHER,
        Request::ReceiveDrop {
            offer,
            mime: mime(TEXT),
        },
    )
    .unwrap();
    assert_eq!(d.h.scene.take_transfer(), Some(APP));
    let events = logged(&mut d.h);
    assert_eq!(
        drag_events(&to(&events, APP)),
        [
            Event::DragSend {
                drag: 7,
                mime: mime(URIS)
            },
            Event::DragSend {
                drag: 7,
                mime: mime(TEXT)
            }
        ]
    );
    // Finished: the source is told, and nothing more is received.
    d.h.request(OTHER, Request::FinishDrop { offer }).unwrap();
    assert_eq!(
        drag_events(&to(&logged(&mut d.h), APP)),
        [Event::DragFinished {
            drag: 7,
            action: DragActions::MOVE
        }]
    );
    d.h.request(
        OTHER,
        Request::ReceiveDrop {
            offer,
            mime: mime(URIS),
        },
    )
    .unwrap();
    assert_eq!(d.h.scene.take_transfer(), None);
}

#[test]
fn escape_cancels_the_drag_with_nothing_transferred_and_reaches_nobody() {
    let mut d = desk();
    start(&mut d, &[URIS], DragActions::COPY);
    let (x, y) = center(&d.h, d.target);
    d.h.scene.pointer_motion(x, y);
    let (offer, serial) = entering(&to(&logged(&mut d.h), OTHER)).unwrap();
    accept(&mut d.h, OTHER, offer, serial, URIS);
    d.h.clear_log();
    d.h.scene.key(ESCAPE, true);
    d.h.scene.key(ESCAPE, false);
    let events = logged(&mut d.h);
    assert_eq!(
        drag_events(&to(&events, OTHER)),
        [Event::DragLeave { surface: 1, offer }]
    );
    assert_eq!(
        drag_events(&to(&events, APP)),
        [Event::DragCancelled {
            drag: 7,
            reason: DragCancelReason::Escape
        }]
    );
    assert!(events
        .iter()
        .all(|(_, event)| !matches!(event, Event::Key { .. })));
    // The release that follows is no click and no drop.
    d.h.scene.pointer_button(BUTTON, false);
    let events = logged(&mut d.h);
    assert!(events.iter().all(|(_, event)| !matches!(
        event,
        Event::Pointer {
            kind: PointerKind::Release,
            ..
        } | Event::DragDrop { .. }
    )));
    d.h.request(
        OTHER,
        Request::ReceiveDrop {
            offer,
            mime: mime(URIS),
        },
    )
    .unwrap();
    assert_eq!(d.h.scene.take_transfer(), None, "nothing transferred");
}

#[test]
fn a_release_over_nothing_that_takes_the_drag_cancels_it() {
    let mut d = desk();
    start(&mut d, &[URIS], DragActions::COPY);
    let (x, y) = center(&d.h, d.target);
    d.h.scene.pointer_motion(x, y);
    d.h.clear_log();
    d.h.scene.pointer_button(BUTTON, false);
    let events = logged(&mut d.h);
    assert!(matches!(
        drag_events(&to(&events, OTHER))[..],
        [Event::DragLeave { .. }]
    ));
    assert_eq!(
        drag_events(&to(&events, APP)),
        [Event::DragCancelled {
            drag: 7,
            reason: DragCancelReason::NotAccepted
        }]
    );
    // The pointer is the person's again: it enters the window under it.
    assert!(to(&events, OTHER).iter().any(|event| matches!(
        event,
        Event::Pointer {
            kind: PointerKind::Enter,
            ..
        }
    )));
}

#[test]
fn the_source_cancelling_destroying_its_window_leaving_or_the_lock_end_the_drag() {
    // Its own cancel.
    let mut d = desk();
    start(&mut d, &[URIS], DragActions::COPY);
    d.h.request(APP, Request::CancelDrag { drag: 7 }).unwrap();
    assert!(to(&logged(&mut d.h), APP).contains(&Event::DragCancelled {
        drag: 7,
        reason: DragCancelReason::Cancelled
    }));
    assert!(!d.h.scene.dragging());
    // The window it began from destroyed.
    let mut d = desk();
    start(&mut d, &[URIS], DragActions::COPY);
    d.h.request(APP, Request::Destroy { surface: 1 }).unwrap();
    assert!(!d.h.scene.dragging());
    // The lock.
    let mut d = desk();
    start(&mut d, &[URIS], DragActions::COPY);
    d.h.scene.lock();
    assert!(to(&logged(&mut d.h), APP).contains(&Event::DragCancelled {
        drag: 7,
        reason: DragCancelReason::Locked
    }));
    // The source gone: its target is told the drag left.
    let mut d = desk();
    start(&mut d, &[URIS], DragActions::COPY);
    let (x, y) = center(&d.h, d.target);
    d.h.scene.pointer_motion(x, y);
    d.h.clear_log();
    d.h.scene.disconnect(APP);
    assert!(matches!(
        drag_events(&to(&logged(&mut d.h), OTHER))[..],
        [Event::DragLeave { .. }]
    ));
    assert!(!d.h.scene.dragging());
}

#[test]
fn a_target_leaving_after_the_drop_abandons_it_and_old_drops_are_pushed_out() {
    let mut d = desk();
    drop_on_target(&mut d, &[URIS]);
    d.h.clear_log();
    d.h.scene.disconnect(OTHER);
    assert!(to(&logged(&mut d.h), APP).contains(&Event::DragCancelled {
        drag: 7,
        reason: DragCancelReason::Abandoned
    }));
    // Drops never finished are pushed out past the bound, the oldest first.
    let mut d = desk();
    for _ in 0..=MAX_OPEN_DROPS {
        drop_on_target(&mut d, &[URIS]);
    }
    let cancelled = to(&logged(&mut d.h), APP)
        .into_iter()
        .filter(|event| {
            matches!(
                event,
                Event::DragCancelled {
                    reason: DragCancelReason::Abandoned,
                    ..
                }
            )
        })
        .count();
    assert_eq!(cancelled, 1);
}

#[test]
fn the_icon_follows_the_pointer_takes_no_input_and_shows_only_during_its_drag() {
    let mut d = desk();
    d.h.request(
        APP,
        Request::CreateDragIcon {
            surface: 9,
            size: Size::new(40, 30),
            hotspot_x: 10,
            hotspot_y: 5,
        },
    )
    .unwrap();
    let icon = key(APP, 9);
    d.h.submit(icon);
    d.h.frame();
    assert!(!d.h.rendered(icon), "no drag names it yet");
    let (x, y) = center(&d.h, d.source);
    let serial = d.h.press(x, y);
    d.h.request(
        APP,
        Request::DragType {
            drag: 7,
            mime: mime(URIS),
        },
    )
    .unwrap();
    d.h.request(
        APP,
        Request::StartDrag {
            drag: 7,
            surface: 1,
            icon: 9,
            serial,
            actions: DragActions::COPY,
        },
    )
    .unwrap();
    assert_eq!(d.h.rect(icon), Rect::new(x - 10, y - 5, 40, 30));
    // Over the target, the target is what is under the pointer, not the icon; the icon is on top.
    let (tx, ty) = center(&d.h, d.target);
    d.h.scene.pointer_motion(tx, ty);
    assert_eq!(d.h.rect(icon), Rect::new(tx - 10, ty - 5, 40, 30));
    assert!(entering(&to(&logged(&mut d.h), OTHER)).is_some());
    let last = d.h.scene.render_list().last().cloned();
    assert!(matches!(last, Some(RenderItem::Surface { key, .. }) if key == icon));
    // Once the drag ends, it is no longer shown.
    d.h.scene.pointer_button(BUTTON, false);
    assert!(!d.h.rendered(icon));
}

#[test]
fn a_client_of_version_9_is_no_target_and_is_told_nothing() {
    let mut d = desk();
    d.h.scene.set_version(OTHER, DRAG_VERSION - 1);
    start(&mut d, &[URIS], DragActions::COPY);
    let (x, y) = center(&d.h, d.target);
    d.h.scene.pointer_motion(x, y);
    d.h.scene.pointer_button(BUTTON, false);
    let events = logged(&mut d.h);
    assert!(drag_events(&to(&events, OTHER)).is_empty());
    assert!(to(&events, APP).contains(&Event::DragCancelled {
        drag: 7,
        reason: DragCancelReason::NotAccepted
    }));
}

#[test]
fn the_shells_files_for_the_trash_are_taken_with_whether_it_cited_its_input() {
    let mut h = Harness::new();
    let dock = h.toplevel(SHELL, 1, 400, 300);
    let (x, y) = center(&h, dock);
    let serial = h.press(x, y);
    for (input, uris) in [
        (serial, "file:///a.txt\nfile:///b.txt\n"),
        (0, "file:///c.txt"),
    ] {
        h.request(
            SHELL,
            Request::TrashFiles {
                input,
                index: 0,
                last: true,
                text: PreeditText::new(uris).unwrap(),
            },
        )
        .unwrap();
    }
    assert_eq!(
        h.scene.take_trash_files(),
        [
            TrashRequest {
                uris: vec!["file:///a.txt".into(), "file:///b.txt".into()],
                with_input: true
            },
            TrashRequest {
                uris: vec!["file:///c.txt".into()],
                with_input: false
            }
        ]
    );
    // Only the shell.
    assert_eq!(
        h.request(
            APP,
            Request::TrashFiles {
                input: 0,
                index: 0,
                last: true,
                text: PreeditText::new("file:///a").unwrap(),
            },
        ),
        Err(ErrorCode::Unauthorized)
    );
}
