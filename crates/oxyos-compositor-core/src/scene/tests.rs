use super::*;
use oxyos_protocol::{
    AccessibleAction, AccessibleField, AccessibleRole, AccessibleState, Adjust, Anchor, Gravity,
    PreeditText, ReaderCommand, TextChunk, TextHints, TextPurpose, MAX_ACCESSIBLE_NODES,
    MAX_ACCESSIBLE_TEXT_BYTES, MAX_PREFERENCE_BYTES, MAX_SURROUNDING_BYTES, NO_ACCESSIBLE_PARENT,
};
use std::collections::HashMap;

mod outputs;

const APP: ClientId = 1;
const OTHER: ClientId = 2;
const SHELL: ClientId = 3;
const LOCKER: ClientId = 4;
/// An assistive technology (a screen reader), connected by the tests that need one.
const READER: ClientId = 5;
const SCREEN: Rect = Rect::new(0, 0, 1920, 1080);
const BUTTON: u32 = 272;
const ESCAPE: u32 = 1;
const SUPER: u32 = 125;

struct Harness {
    scene: Scene,
    configures: HashMap<SurfaceKey, (u64, Rect, u32)>,
    log: Vec<(ClientId, Event)>,
    next_buffer: u64,
    next_sequence: HashMap<SurfaceKey, u64>,
    /// Released buffers each client may reuse, with their extent.
    free: HashMap<ClientId, Vec<(u64, Size)>>,
    extents: HashMap<(ClientId, u64), Size>,
}

fn key(client: ClientId, id: u32) -> SurfaceKey {
    SurfaceKey { client, id }
}

impl Harness {
    fn new() -> Self {
        let mut scene = Scene::new();
        scene
            .add_output(Output {
                id: 1,
                rect: SCREEN,
                scale_120: 120,
                refresh_mhz: 60_000,
            })
            .unwrap();
        for (client, grants) in [
            (APP, Grants::default()),
            (OTHER, Grants::default()),
            (
                SHELL,
                Grants {
                    shell: true,
                    shortcuts: true,
                    ..Grants::default()
                },
            ),
            (
                LOCKER,
                Grants {
                    lock: true,
                    ..Grants::default()
                },
            ),
        ] {
            scene.connect(client, grants).unwrap();
        }
        let mut harness = Self {
            scene,
            configures: HashMap::new(),
            log: Vec::new(),
            next_buffer: 1,
            next_sequence: HashMap::new(),
            free: HashMap::new(),
            extents: HashMap::new(),
        };
        harness.scene.set_shortcuts([SUPER]);
        harness
    }

    fn pump(&mut self) -> Vec<(ClientId, Event)> {
        let events = self.scene.drain_events();
        for (client, event) in &events {
            if let Event::Configure {
                surface,
                serial,
                rect,
                scale_120,
                ..
            } = event
            {
                self.configures
                    .insert(key(*client, *surface), (*serial, *rect, *scale_120));
            }
            if let Event::Released { buffer, .. } = event {
                if let Some(extent) = self.extents.get(&(*client, *buffer)) {
                    self.free
                        .entry(*client)
                        .or_default()
                        .push((*buffer, *extent));
                }
            }
        }
        self.log.extend(events.iter().copied());
        events
    }

    fn request(&mut self, client: ClientId, request: Request) -> Result<(), ErrorCode> {
        let result = self.scene.handle(client, request);
        self.pump();
        result
    }

    /// Registers a new buffer as if the transport had validated its memory object.
    fn attach(&mut self, client: ClientId, buffer: u64, extent: Size) {
        self.request(
            client,
            Request::AttachBuffer {
                buffer,
                format: oxyos_protocol::Format::Xrgb8888,
                extent,
                stride: extent.width * 4,
                offset: 0,
            },
        )
        .unwrap();
        self.extents.insert((client, buffer), extent);
    }

    /// A buffer of `extent`: a released one when available, otherwise a new one.
    fn buffer(&mut self, client: ClientId, extent: Size) -> u64 {
        let free = self.free.entry(client).or_default();
        if let Some(index) = free.iter().position(|(_, e)| *e == extent) {
            return free.swap_remove(index).0;
        }
        let buffer = self.next_buffer;
        self.next_buffer += 1;
        self.attach(client, buffer, extent);
        buffer
    }

    /// Destroys every released buffer, as a client does when it shrinks its pool.
    fn destroy_free(&mut self, client: ClientId) {
        for (buffer, _) in self.free.remove(&client).unwrap_or_default() {
            self.request(client, Request::DestroyBuffer { buffer })
                .unwrap();
        }
    }

    fn submit(&mut self, surface: SurfaceKey) -> u64 {
        self.pump();
        let (serial, rect, scale) = self.configures[&surface];
        let extent = physical(Size::new(rect.width, rect.height), scale);
        let buffer = self.buffer(surface.client, extent);
        let sequence = self.next_sequence.entry(surface).or_insert(0);
        *sequence += 1;
        let request = Request::Submit {
            surface: surface.id,
            sequence: *sequence,
            buffer,
            configure: serial,
            extent,
        };
        self.request(surface.client, request).unwrap();
        buffer
    }

    fn frame(&mut self) {
        self.scene.begin_frame();
        self.scene.end_frame(1, Clock::Synthetic);
        self.pump();
    }

    fn present(&mut self, surface: SurfaceKey) {
        self.submit(surface);
        self.frame();
    }

    fn toplevel(&mut self, client: ClientId, id: u32, width: u32, height: u32) -> SurfaceKey {
        self.request(
            client,
            Request::CreateToplevel {
                surface: id,
                size: Size::new(width, height),
            },
        )
        .unwrap();
        self.present(key(client, id));
        key(client, id)
    }

    fn click(&mut self, x: i32, y: i32) {
        self.scene.pointer_motion(x, y);
        self.scene.pointer_button(BUTTON, true);
        self.scene.pointer_button(BUTTON, false);
        self.pump();
    }

    fn press(&mut self, x: i32, y: i32) -> u64 {
        self.scene.pointer_motion(x, y);
        self.scene.pointer_button(BUTTON, true);
        self.pump()
            .into_iter()
            .find_map(|(_, event)| match event {
                Event::Pointer {
                    kind: PointerKind::Press,
                    serial,
                    ..
                } => Some(serial),
                _ => None,
            })
            .expect("press delivered")
    }

    fn rect(&self, surface: SurfaceKey) -> Rect {
        self.scene
            .render_list()
            .into_iter()
            .find_map(|item| match item {
                RenderItem::Surface { key, rect, .. } if key == surface => Some(rect),
                _ => None,
            })
            .expect("surface rendered")
    }

    fn rendered(&self, surface: SurfaceKey) -> bool {
        self.scene
            .render_list()
            .iter()
            .any(|item| matches!(item, RenderItem::Surface { key, .. } if *key == surface))
    }

    fn popup(&mut self, parent: SurfaceKey, id: u32, positioner: Positioner) -> SurfaceKey {
        self.request(
            parent.client,
            Request::CreatePopup {
                surface: id,
                parent: parent.id,
                positioner,
            },
        )
        .unwrap();
        key(parent.client, id)
    }

    /// Opens a grabbing menu from a press on `trigger` (global coordinates) of `parent`.
    fn open_menu(
        &mut self,
        parent: SurfaceKey,
        id: u32,
        trigger: (i32, i32),
        positioner: Positioner,
    ) -> SurfaceKey {
        let serial = self.press(trigger.0, trigger.1);
        let popup = self.popup(parent, id, positioner);
        self.request(
            parent.client,
            Request::GrabPopup {
                surface: id,
                serial,
            },
        )
        .unwrap();
        self.present(popup);
        self.scene.pointer_button(BUTTON, false);
        self.pump();
        popup
    }

    fn events_for(&self, client: ClientId) -> Vec<Event> {
        self.log
            .iter()
            .filter(|(c, _)| *c == client)
            .map(|(_, e)| *e)
            .collect()
    }

    fn dismissed(&self, surface: SurfaceKey) -> Option<DismissReason> {
        self.log.iter().find_map(|(client, event)| match event {
            Event::PopupDone {
                surface: id,
                reason,
            } if *client == surface.client && *id == surface.id => Some(*reason),
            _ => None,
        })
    }

    fn clear_log(&mut self) {
        self.pump();
        self.log.clear();
    }
}

fn menu(anchor_rect: Rect, width: u32, height: u32) -> Positioner {
    Positioner {
        anchor_rect,
        size: Size::new(width, height),
        anchor: Anchor::BottomLeft,
        gravity: Gravity::BottomRight,
        adjust: Adjust::FLIP_X
            .union(Adjust::FLIP_Y)
            .union(Adjust::SLIDE_X)
            .union(Adjust::SLIDE_Y)
            .union(Adjust::RESIZE_X)
            .union(Adjust::RESIZE_Y),
        offset_x: 0,
        offset_y: 0,
    }
}

fn pointer_presses(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|e| {
            matches!(
                e,
                Event::Pointer {
                    kind: PointerKind::Press | PointerKind::Release,
                    ..
                }
            )
        })
        .count()
}

#[test]
fn a_menu_surface_extends_beyond_all_four_edges_of_its_window() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 200, 120);
    let parent = h.rect(window);
    assert_eq!(parent, Rect::new(860, 480, 200, 120));
    // Centered positioner: the menu is larger than the window in both directions.
    let mut positioner = menu(Rect::new(90, 50, 20, 20), 600, 400);
    positioner.anchor = Anchor::Center;
    positioner.gravity = Gravity::Center;
    let popup = h.open_menu(window, 2, (950, 540), positioner);
    let placed = h.rect(popup);
    assert!(i64::from(placed.x) < i64::from(parent.x), "left");
    assert!(placed.y < parent.y, "top");
    assert!(placed.right() > parent.right(), "right");
    assert!(placed.bottom() > parent.bottom(), "bottom");
    assert!(SCREEN.contains(placed));
    // It is a separate surface stacked directly above its window.
    let order: Vec<SurfaceKey> = h
        .scene
        .render_list()
        .into_iter()
        .filter_map(|item| match item {
            RenderItem::Surface { key, .. } => Some(key),
            _ => None,
        })
        .collect();
    assert_eq!(order, vec![window, popup]);
}

#[test]
fn popups_are_only_shown_with_ready_content() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let serial = h.press(1000, 500);
    let popup = h.popup(window, 2, menu(Rect::new(10, 10, 20, 20), 100, 100));
    h.request(APP, Request::GrabPopup { surface: 2, serial })
        .unwrap();
    h.frame();
    assert!(
        !h.rendered(popup),
        "no empty rectangle before the first frame"
    );
    h.submit(popup);
    assert!(!h.rendered(popup), "submitted but not yet presented");
    h.frame();
    assert!(h.rendered(popup));
}

#[test]
fn identifiers_are_connection_local_so_cross_client_parenting_is_impossible() {
    let mut h = Harness::new();
    h.toplevel(APP, 1, 400, 300);
    // OTHER names id 1, which only exists for APP.
    assert_eq!(
        h.request(
            OTHER,
            Request::CreatePopup {
                surface: 2,
                parent: 1,
                positioner: menu(Rect::new(0, 0, 10, 10), 50, 50)
            }
        ),
        Err(ErrorCode::InvalidParent)
    );
    // A popup cannot parent itself or a surface that does not exist yet (no cycles).
    assert_eq!(
        h.request(
            APP,
            Request::CreatePopup {
                surface: 3,
                parent: 3,
                positioner: menu(Rect::new(0, 0, 10, 10), 50, 50)
            }
        ),
        Err(ErrorCode::InvalidParent)
    );
    // Unmapped parents and anchors outside the parent are rejected.
    h.request(
        APP,
        Request::CreateToplevel {
            surface: 4,
            size: Size::new(10, 10),
        },
    )
    .unwrap();
    assert_eq!(
        h.request(
            APP,
            Request::CreatePopup {
                surface: 5,
                parent: 4,
                positioner: menu(Rect::new(0, 0, 5, 5), 50, 50)
            }
        ),
        Err(ErrorCode::InvalidParent)
    );
    assert_eq!(
        h.request(
            APP,
            Request::CreatePopup {
                surface: 5,
                parent: 1,
                positioner: menu(Rect::new(390, 290, 20, 20), 50, 50)
            }
        ),
        Err(ErrorCode::InvalidPositioner)
    );
}

#[test]
fn privileged_roles_require_session_grants_and_roles_never_change() {
    let mut h = Harness::new();
    let panel = Request::CreateShellSurface {
        surface: 1,
        kind: ShellKind::Panel,
        edge: PanelEdge::Top,
        output: 1,
        size: Size::new(1, 32),
        exclusive: 32,
    };
    assert_eq!(h.request(APP, panel), Err(ErrorCode::Unauthorized));
    assert_eq!(
        h.request(
            APP,
            Request::CreateLockSurface {
                surface: 1,
                output: 1
            }
        ),
        Err(ErrorCode::Unauthorized)
    );
    assert_eq!(
        h.request(
            SHELL,
            Request::CreateLockSurface {
                surface: 9,
                output: 1
            }
        ),
        Err(ErrorCode::Unauthorized),
        "shell grants do not include the lock"
    );
    h.request(SHELL, panel).unwrap();
    assert_eq!(h.request(SHELL, panel), Err(ErrorCode::SurfaceExists));
    let window = h.toplevel(APP, 1, 100, 100);
    for request in [
        Request::GrabPopup {
            surface: 1,
            serial: 1,
        },
        Request::Reposition {
            surface: 1,
            positioner: menu(Rect::new(0, 0, 1, 1), 1, 1),
        },
    ] {
        assert_eq!(h.request(APP, request), Err(ErrorCode::InvalidRole));
    }
    assert_eq!(
        h.request(
            APP,
            Request::CreateDialog {
                surface: 2,
                parent: 99,
                size: Size::new(10, 10)
            }
        ),
        Err(ErrorCode::InvalidParent)
    );
    let _ = window;
    assert_eq!(
        h.request(
            APP,
            Request::Hello {
                min_version: 2,
                max_version: 2
            }
        ),
        Err(ErrorCode::InvalidOrder)
    );
}

#[test]
fn popup_depth_and_surface_quotas_are_bounded() {
    let mut h = Harness::new();
    let mut parent = h.toplevel(APP, 1, 800, 600);
    for id in 2..2 + MAX_POPUP_DEPTH as u32 {
        let popup = h.popup(parent, id, menu(Rect::new(0, 0, 10, 10), 40, 40));
        h.present(popup);
        parent = popup;
    }
    assert_eq!(
        h.request(
            APP,
            Request::CreatePopup {
                surface: 100,
                parent: parent.id,
                positioner: menu(Rect::new(0, 0, 10, 10), 40, 40)
            }
        ),
        Err(ErrorCode::QuotaExceeded)
    );
    let mut h = Harness::new();
    for id in 1..=MAX_SURFACES_PER_CLIENT as u32 {
        h.request(
            OTHER,
            Request::CreateToplevel {
                surface: id,
                size: Size::new(10, 10),
            },
        )
        .unwrap();
    }
    assert_eq!(
        h.request(
            OTHER,
            Request::CreateToplevel {
                surface: 999,
                size: Size::new(10, 10)
            }
        ),
        Err(ErrorCode::QuotaExceeded)
    );
    // Other clients are unaffected by one client's exhaustion.
    h.toplevel(APP, 1, 10, 10);
}

#[test]
fn grabs_need_the_latest_press_on_the_parent_chain() {
    let mut h = Harness::new();
    // OTHER spans (360,140)-(1560,940); APP's window sits on top at (760,390)-(1160,690).
    let other = h.toplevel(OTHER, 1, 1200, 800);
    let window = h.toplevel(APP, 1, 400, 300);
    let stale = h.press(800, 400);
    h.scene.pointer_button(BUTTON, false);
    h.press(800, 400);
    h.scene.pointer_button(BUTTON, false);
    let popup = h.popup(window, 2, menu(Rect::new(0, 0, 10, 10), 50, 50));
    h.request(
        APP,
        Request::GrabPopup {
            surface: 2,
            serial: stale,
        },
    )
    .unwrap();
    assert_eq!(h.dismissed(popup), Some(DismissReason::GrabDenied));
    assert!(h.scene.grab_chain().is_empty());

    // A press delivered to another client cannot authorize this client's grab.
    let other_rect = h.rect(other);
    let foreign = h.press(other_rect.x + 40, other_rect.y + 60);
    h.scene.pointer_button(BUTTON, false);
    let popup = h.popup(window, 3, menu(Rect::new(0, 0, 10, 10), 50, 50));
    h.request(
        APP,
        Request::GrabPopup {
            surface: 3,
            serial: foreign,
        },
    )
    .unwrap();
    assert_eq!(h.dismissed(popup), Some(DismissReason::GrabDenied));

    // A key press is a valid activation (keyboard-opened menu). The foreign press raised OTHER.
    h.click(h.rect(window).x + 5, h.rect(window).y + 5);
    assert_eq!(h.scene.focus(), Some(other), "OTHER now covers that point");
    h.activate(window);
    h.scene.key(28, true);
    let serial = h
        .pump()
        .into_iter()
        .find_map(|(_, e)| match e {
            Event::Key { serial, .. } => Some(serial),
            _ => None,
        })
        .unwrap();
    let popup = h.popup(window, 4, menu(Rect::new(0, 0, 10, 10), 50, 50));
    h.request(APP, Request::GrabPopup { surface: 4, serial })
        .unwrap();
    assert_eq!(h.scene.grab_chain(), &[popup]);
    assert_eq!(h.scene.focus(), Some(popup));
}

#[test]
fn nested_submenus_chain_and_unwind_with_focus_return() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let w = h.rect(window);
    let menu_popup = h.open_menu(
        window,
        2,
        (w.x + 10, w.y + 10),
        menu(Rect::new(0, 0, 40, 20), 200, 300),
    );
    assert_eq!(h.scene.focus(), Some(menu_popup));
    let m = h.rect(menu_popup);
    // Activating a row of the menu opens a submenu to its right.
    let serial = h.press(m.x + 20, m.y + 40);
    let mut positioner = menu(Rect::new(0, 30, 200, 20), 180, 200);
    positioner.anchor = Anchor::TopRight;
    positioner.gravity = Gravity::BottomRight;
    let sub = h.popup(menu_popup, 3, positioner);
    h.request(APP, Request::GrabPopup { surface: 3, serial })
        .unwrap();
    h.present(sub);
    h.scene.pointer_button(BUTTON, false);
    assert_eq!(h.scene.grab_chain(), &[menu_popup, sub]);
    assert_eq!(h.scene.focus(), Some(sub));
    assert_eq!(h.rect(sub).x, m.x + 200);

    // A grab for a sibling of the submenu is not the top of the chain.
    let serial = h.press(m.x + 20, m.y + 60);
    let sibling = h.popup(menu_popup, 4, menu(Rect::new(0, 50, 20, 20), 50, 50));
    h.request(APP, Request::GrabPopup { surface: 4, serial })
        .unwrap();
    assert_eq!(h.dismissed(sibling), Some(DismissReason::GrabDenied));
    h.scene.pointer_button(BUTTON, false);

    // Escape is application semantics: the client closes the submenu by destroying it.
    h.scene.key(ESCAPE, true);
    assert!(h.pump().iter().any(|(c, e)| *c == APP
        && matches!(
            e,
            Event::Key {
                surface: 3,
                key: ESCAPE,
                ..
            }
        )));
    h.request(APP, Request::Destroy { surface: 3 }).unwrap();
    assert_eq!(h.scene.grab_chain(), &[menu_popup]);
    assert_eq!(h.scene.focus(), Some(menu_popup));
    h.request(APP, Request::Destroy { surface: 2 }).unwrap();
    assert!(h.scene.grab_chain().is_empty());
    assert_eq!(h.scene.focus(), Some(window), "focus returns to the window");
}

#[test]
fn outside_press_dismisses_the_chain_and_is_consumed() {
    let mut h = Harness::new();
    let other = h.toplevel(OTHER, 1, 1200, 800);
    let window = h.toplevel(APP, 1, 400, 300);
    let w = h.rect(window);
    let popup = h.open_menu(
        window,
        2,
        (w.x + 10, w.y + 10),
        menu(Rect::new(0, 0, 40, 20), 200, 150),
    );
    h.clear_log();
    // Press on the other application: the menu closes and the other app sees nothing.
    let o = h.rect(other);
    let (x, y) = (o.x + 40, o.y + 60);
    assert!(!h.rect(popup).contains_point(x, y));
    h.click(x, y);
    assert_eq!(h.dismissed(popup), Some(DismissReason::OutsidePress));
    assert_eq!(
        pointer_presses(&h.events_for(OTHER)),
        0,
        "press and release consumed"
    );
    assert_eq!(
        h.scene.focus(),
        Some(window),
        "the dismissing press does not activate"
    );
    // The next press is ordinary input again.
    h.click(x, y);
    assert_eq!(pointer_presses(&h.events_for(OTHER)), 2);
}

#[test]
fn pressing_the_trigger_again_closes_without_reopening() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let w = h.rect(window);
    let trigger = (w.x + 10, w.y + 10);
    let popup = h.open_menu(window, 2, trigger, menu(Rect::new(0, 0, 40, 20), 200, 150));
    h.clear_log();
    h.click(trigger.0, trigger.1);
    assert_eq!(h.dismissed(popup), Some(DismissReason::OutsidePress));
    assert_eq!(
        pointer_presses(&h.events_for(APP)),
        0,
        "the trigger never sees the press"
    );
}

#[test]
fn menu_input_works_over_another_applications_region() {
    let mut h = Harness::new();
    let other = h.toplevel(OTHER, 1, 1200, 800);
    let window = h.toplevel(APP, 1, 200, 100);
    let w = h.rect(window);
    let popup = h.open_menu(
        window,
        2,
        (w.x + 5, w.y + 90),
        menu(Rect::new(0, 80, 40, 20), 300, 400),
    );
    let p = h.rect(popup);
    let (x, y) = (p.x + 150, p.y + 300);
    assert!(!w.contains_point(x, y) && h.rect(other).contains_point(x, y));
    h.clear_log();
    h.scene.pointer_motion(x, y);
    h.scene.pointer_button(BUTTON, true);
    h.scene.pointer_button(BUTTON, false);
    h.pump();
    assert_eq!(pointer_presses(&h.events_for(APP)), 2);
    assert!(
        h.events_for(OTHER).is_empty(),
        "the covered application receives nothing"
    );
    assert!(h.events_for(APP).iter().any(|e| matches!(
        e,
        Event::Pointer {
            surface: 2,
            kind: PointerKind::Release,
            x: 38_400,
            y: 76_800,
            ..
        }
    )));
    assert_eq!(h.dismissed(popup), None);
}

#[test]
fn press_drag_release_selects_in_the_menu() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let w = h.rect(window);
    let serial = h.press(w.x + 10, w.y + 10);
    let popup = h.popup(window, 2, menu(Rect::new(0, 0, 40, 20), 200, 150));
    h.request(APP, Request::GrabPopup { surface: 2, serial })
        .unwrap();
    h.present(popup);
    h.clear_log();
    let p = h.rect(popup);
    h.scene.pointer_motion(p.x + 50, p.y + 50);
    h.scene.pointer_button(BUTTON, false);
    h.pump();
    assert!(h.events_for(APP).iter().any(|e| matches!(
        e,
        Event::Pointer {
            surface: 2,
            kind: PointerKind::Release,
            ..
        }
    )));
    assert_eq!(h.dismissed(popup), None);
}

#[test]
fn parent_move_carries_popups_in_the_same_frame_then_reconstrains() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let w = h.rect(window);
    // Open a non-grabbing popup (e.g. a pinned menu) to keep the pointer free for the move.
    let popup = h.popup(window, 2, menu(Rect::new(0, 280, 40, 20), 200, 150));
    h.present(popup);
    let before = h.rect(popup);
    let serial = h.press(w.x + 200, w.y + 10);
    h.request(APP, Request::BeginMove { surface: 1, serial })
        .unwrap();
    // The client is now stalled: it sends nothing while the window moves.
    h.clear_log();
    h.scene.pointer_motion(w.x + 200 + 100, w.y + 10 + 50);
    let moved = h.rect(window);
    assert_eq!((moved.x, moved.y), (w.x + 100, w.y + 50));
    let carried = h.rect(popup);
    assert_eq!(
        (carried.x - before.x, carried.y - before.y),
        (100, 50),
        "no detached jump"
    );
    // Drag down until the popup would leave the bottom edge; on release it is re-placed.
    h.scene.pointer_motion(w.x + 200 + 100, 1000);
    h.scene.pointer_button(BUTTON, false);
    h.pump();
    let (_, rect, _) = h.configures[&popup];
    assert!(rect.y < 0, "flipped above the anchor: {rect:?}");
    assert!(
        h.events_for(APP).iter().all(|e| !matches!(
            e,
            Event::Pointer {
                kind: PointerKind::Motion | PointerKind::Press | PointerKind::Release,
                ..
            }
        )),
        "the move is compositor-owned"
    );
    h.present(popup);
    let flipped = h.rect(popup);
    assert!(SCREEN.contains(flipped), "{flipped:?}");
}

#[test]
fn parent_minimize_destroy_and_disconnect_dismiss_popups_and_release_grabs() {
    for ending in 0..3 {
        let mut h = Harness::new();
        let window = h.toplevel(APP, 1, 400, 300);
        let w = h.rect(window);
        let popup = h.open_menu(
            window,
            2,
            (w.x + 10, w.y + 10),
            menu(Rect::new(0, 0, 40, 20), 200, 150),
        );
        match ending {
            0 => {
                h.scene.minimize(window);
                h.pump();
                assert_eq!(h.dismissed(popup), Some(DismissReason::ParentHidden));
            }
            1 => {
                h.request(APP, Request::Destroy { surface: 1 }).unwrap();
                assert_eq!(h.dismissed(popup), Some(DismissReason::ParentGone));
                // The orphan stays addressable until the client destroys it.
                h.request(APP, Request::Destroy { surface: 2 }).unwrap();
            }
            _ => {
                h.scene.disconnect(APP);
                h.pump();
            }
        }
        assert!(h.scene.grab_chain().is_empty());
        assert!(!h.rendered(popup));
        assert_eq!(
            h.scene.stats().held_buffers,
            if ending == 0 { 1 } else { 0 }
        );
    }
}

#[test]
fn shell_failure_does_not_strand_application_grabs() {
    let mut h = Harness::new();
    h.request(
        SHELL,
        Request::CreateShellSurface {
            surface: 1,
            kind: ShellKind::Panel,
            edge: PanelEdge::Top,
            output: 1,
            size: Size::new(1, 32),
            exclusive: 32,
        },
    )
    .unwrap();
    h.present(key(SHELL, 1));
    let window = h.toplevel(APP, 1, 400, 300);
    let w = h.rect(window);
    let popup = h.open_menu(
        window,
        2,
        (w.x + 10, w.y + 10),
        menu(Rect::new(0, 0, 40, 20), 200, 150),
    );
    h.scene.disconnect(SHELL);
    h.pump();
    assert_eq!(h.scene.grab_chain(), &[popup]);
    assert_eq!(
        h.scene.work_area(1),
        Some(SCREEN),
        "the panel's exclusive zone is gone"
    );
    h.click(5, 5);
    assert_eq!(h.dismissed(popup), Some(DismissReason::OutsidePress));
}

#[test]
fn repeated_lifecycles_settle_resource_counts() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let w = h.rect(window);
    h.pump();
    let baseline = h.scene.stats();
    for round in 0..500_u32 {
        let id = 10 + round;
        let popup = h.open_menu(
            window,
            id,
            (w.x + 10, w.y + 10),
            menu(Rect::new(0, 0, 40, 20), 200, 150),
        );
        h.present(popup);
        h.submit(popup);
        match round % 3 {
            0 => h.request(APP, Request::Destroy { surface: id }).unwrap(),
            1 => {
                h.click(1900, 1070);
                h.request(APP, Request::Destroy { surface: id }).unwrap();
            }
            _ => {
                // Destroyed while its frame is in flight: release waits for end_frame.
                h.scene.begin_frame();
                h.request(APP, Request::Destroy { surface: id }).unwrap();
                assert!(h.scene.stats().deferred_releases > 0);
                h.scene.end_frame(2, Clock::Synthetic);
            }
        }
        h.scene.drain_retired();
        h.pump();
        h.log.clear();
    }
    h.destroy_free(APP);
    h.scene.drain_retired();
    let after = h.scene.stats();
    assert_eq!(after, baseline);
    assert_eq!(after.surfaces, 1);
    assert_eq!(after.held_buffers, 1);

    // A full client lifecycle settles too.
    for round in 0..200_u32 {
        let client = 100 + round;
        h.scene.connect(client, Grants::default()).unwrap();
        let top = h.toplevel(client, 1, 300, 200);
        h.submit(top);
        let popup = h.popup(top, 2, menu(Rect::new(0, 0, 10, 10), 50, 50));
        h.present(popup);
        h.scene.disconnect(client);
        h.pump();
    }
    let retired = h.scene.drain_retired();
    assert!(retired.iter().all(|(client, _)| *client >= 100));
    assert_eq!(
        retired.len(),
        200 * 3,
        "each client's three buffers are retired once"
    );
    assert_eq!(h.scene.stats(), baseline);
}

#[test]
fn lock_dismisses_popups_covers_outputs_and_survives_lock_client_crash() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let w = h.rect(window);
    let popup = h.open_menu(
        window,
        2,
        (w.x + 10, w.y + 10),
        menu(Rect::new(0, 0, 40, 20), 200, 150),
    );
    h.scene.lock();
    h.pump();
    assert!(h.scene.is_locked());
    assert_eq!(h.dismissed(popup), Some(DismissReason::Locked));
    assert_eq!(
        h.scene.render_list(),
        vec![RenderItem::Cover {
            output: 1,
            rect: SCREEN
        }],
        "protected immediately, before any lock UI is ready"
    );
    assert_eq!(h.scene.focus(), None);
    // Ordinary clients cannot open grabs or receive input while locked.
    h.clear_log();
    h.click(w.x + 10, w.y + 10);
    h.scene.key(30, true);
    h.pump();
    assert!(h.events_for(APP).is_empty());
    h.request(
        LOCKER,
        Request::CreateLockSurface {
            surface: 1,
            output: 1,
        },
    )
    .unwrap();
    h.present(key(LOCKER, 1));
    assert_eq!(h.scene.focus(), Some(key(LOCKER, 1)));
    assert!(h.rendered(key(LOCKER, 1)) && !h.rendered(window));
    // The lock UI crashes: the session stays locked and covered.
    h.scene.disconnect(LOCKER);
    assert!(h.scene.is_locked());
    assert_eq!(
        h.scene.render_list(),
        vec![RenderItem::Cover {
            output: 1,
            rect: SCREEN
        }]
    );
    // Only the trusted path unlocks.
    h.scene.unlock();
    h.pump();
    assert!(h.rendered(window));
    assert_eq!(h.scene.focus(), Some(window));
}

#[test]
fn activity_counts_the_persons_input_and_nothing_else() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let w = h.rect(window);
    let before = h.scene.activity();
    // What clients do and frames shown are not the person's input.
    h.present(window);
    h.scene.begin_frame();
    h.scene.end_frame(0, Clock::Synthetic);
    h.pump();
    assert_eq!(h.scene.activity(), before);
    // Every kind of input counts, wherever it goes: a window, nothing, the lock screen's covers.
    let inputs: [&dyn Fn(&mut Scene); 11] = [
        &|scene| scene.pointer_motion(w.x + 5, w.y + 5),
        &|scene| scene.pointer_button(BUTTON, true),
        &|scene| scene.pointer_button(BUTTON, false),
        &|scene| scene.pointer_scroll(0, 256),
        &|scene| scene.pointer_pinch(GesturePhase::Begin, 65536, 0),
        &|scene| scene.key(30, true),
        &|scene| scene.keyboard(30, false, None),
        &|scene| scene.text("a"),
        &|scene| scene.touch_down(1, 0, 0),
        &|scene| scene.touch_motion(1, 3, 3),
        &|scene| scene.touch_up(1),
    ];
    for locked in [false, true] {
        if locked {
            h.scene.lock();
        }
        for input in inputs {
            let seen = h.scene.activity();
            input(&mut h.scene);
            assert_ne!(h.scene.activity(), seen, "locked: {locked}");
        }
    }
    // Locking and unlocking are not input.
    let seen = h.scene.activity();
    h.scene.unlock();
    h.scene.lock();
    assert_eq!(h.scene.activity(), seen);
}

#[test]
fn shell_panels_reserve_work_area_and_transparent_regions_pass_input_through() {
    let mut h = Harness::new();
    h.request(
        SHELL,
        Request::CreateShellSurface {
            surface: 1,
            kind: ShellKind::Panel,
            edge: PanelEdge::Top,
            output: 1,
            size: Size::new(1, 40),
            exclusive: 32,
        },
    )
    .unwrap();
    h.present(key(SHELL, 1));
    assert_eq!(h.scene.work_area(1), Some(Rect::new(0, 32, 1920, 1048)));
    // The panel's bottom 8 px are a transparent shadow; only the bar takes input.
    h.request(
        SHELL,
        Request::SetInputRegion {
            surface: 1,
            region: Rect::new(0, 0, 1920, 32),
        },
    )
    .unwrap();
    let window = h.toplevel(APP, 1, 1920, 1048);
    assert_eq!(h.rect(window).y, 32);
    h.clear_log();
    h.click(500, 35);
    assert_eq!(pointer_presses(&h.events_for(APP)), 2);
    assert!(h.events_for(SHELL).iter().all(|e| !matches!(
        e,
        Event::Pointer {
            kind: PointerKind::Press,
            ..
        }
    )));
    h.click(500, 10);
    assert_eq!(pointer_presses(&h.events_for(SHELL)), 2);
    // Application menus stay out of the reserved area; shell menus may use the whole output.
    let mut up = menu(Rect::new(0, 0, 10, 10), 100, 2000);
    up.adjust = Adjust::SLIDE_Y.union(Adjust::RESIZE_Y);
    let w = h.rect(window);
    let popup = h.open_menu(window, 2, (w.x + 5, w.y + 5), up);
    assert_eq!(h.rect(popup).y, 32);
    assert_eq!(h.rect(popup).height, 1048);
}

#[test]
fn shell_overlays_compose_above_fullscreen_content_and_block_direct_scanout() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 800, 600);
    h.request(
        APP,
        Request::SetFullscreen {
            surface: 1,
            enabled: true,
        },
    )
    .unwrap();
    h.present(window);
    assert_eq!(h.rect(window), SCREEN);
    assert_eq!(h.scene.scanout_candidate(1), Some(window));
    h.request(
        SHELL,
        Request::CreateShellSurface {
            surface: 1,
            kind: ShellKind::Notification,
            edge: PanelEdge::None,
            output: 1,
            size: Size::new(360, 120),
            exclusive: 0,
        },
    )
    .unwrap();
    h.present(key(SHELL, 1));
    assert_eq!(
        h.scene.scanout_candidate(1),
        None,
        "composition while occluded"
    );
    let order: Vec<SurfaceKey> = h
        .scene
        .render_list()
        .into_iter()
        .filter_map(|item| match item {
            RenderItem::Surface { key, .. } => Some(key),
            _ => None,
        })
        .collect();
    assert_eq!(order, vec![window, key(SHELL, 1)]);
    h.request(SHELL, Request::Destroy { surface: 1 }).unwrap();
    assert_eq!(h.scene.scanout_candidate(1), Some(window));
    // A menu over fullscreen content also forces composition.
    let popup = h.open_menu(
        window,
        2,
        (100, 100),
        menu(Rect::new(90, 90, 20, 20), 200, 200),
    );
    assert!(h.rendered(popup));
    assert_eq!(h.scene.scanout_candidate(1), None);
}

#[test]
fn shortcuts_and_window_moves_do_not_wait_for_a_stalled_application() {
    let mut h = Harness::new();
    h.request(
        SHELL,
        Request::CreateShellSurface {
            surface: 1,
            kind: ShellKind::Panel,
            edge: PanelEdge::Bottom,
            output: 1,
            size: Size::new(1, 48),
            exclusive: 48,
        },
    )
    .unwrap();
    h.present(key(SHELL, 1));
    let window = h.toplevel(APP, 1, 400, 300);
    assert_eq!(h.scene.focus(), Some(window));
    // APP never reads or answers from here on.
    h.clear_log();
    h.scene.key(SUPER, true);
    h.scene.key(SUPER, false);
    h.pump();
    assert_eq!(
        h.events_for(SHELL)
            .iter()
            .filter(|e| matches!(e, Event::Key { key: SUPER, .. }))
            .count(),
        2
    );
    assert!(h
        .events_for(APP)
        .iter()
        .all(|e| !matches!(e, Event::Key { .. })));
}

#[test]
fn stale_configures_are_discarded_and_last_content_is_kept_during_resize() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let shown = h.rect(window);
    let (old_serial, _, _) = h.configures[&window];
    h.request(
        APP,
        Request::SetFullscreen {
            surface: 1,
            enabled: true,
        },
    )
    .unwrap();
    h.attach(APP, 900, Size::new(400, 300));
    h.clear_log();
    // The client was mid-frame for the old size.
    h.request(
        APP,
        Request::Submit {
            surface: 1,
            sequence: 50,
            buffer: 900,
            configure: old_serial,
            extent: Size::new(400, 300),
        },
    )
    .unwrap();
    assert!(h.events_for(APP).contains(&Event::Feedback {
        surface: 1,
        sequence: 50,
        outcome: Outcome::Discarded,
        timestamp_ns: 0,
        clock: Clock::Synthetic
    }));
    assert!(h.events_for(APP).contains(&Event::Released {
        surface: 1,
        buffer: 900
    }));
    h.frame();
    assert_eq!(
        h.rect(window),
        shown,
        "old content stays until the new size is ready"
    );
    h.present(window);
    assert_eq!(h.rect(window), SCREEN);
    assert!(h.log.iter().any(|(_, e)| matches!(
        e,
        Event::Feedback {
            outcome: Outcome::Presented,
            clock: Clock::Synthetic,
            ..
        }
    )));
    // A current-serial submission with the wrong extent or a reused sequence is a violation.
    let (serial, _, _) = h.configures[&window];
    h.attach(APP, 901, Size::new(10, 10));
    h.attach(APP, 902, Size::new(1920, 1080));
    assert_eq!(
        h.request(
            APP,
            Request::Submit {
                surface: 1,
                sequence: 99,
                buffer: 901,
                configure: serial,
                extent: Size::new(10, 10)
            }
        ),
        Err(ErrorCode::InvalidBuffer)
    );
    assert_eq!(
        h.request(
            APP,
            Request::Submit {
                surface: 1,
                sequence: 1,
                buffer: 902,
                configure: serial,
                extent: Size::new(1920, 1080)
            }
        ),
        Err(ErrorCode::InvalidOrder)
    );
}

#[test]
fn touch_outside_a_grab_dismisses_and_cancel_releases_every_point() {
    let mut h = Harness::new();
    let other = h.toplevel(OTHER, 1, 1200, 800);
    let window = h.toplevel(APP, 1, 400, 300);
    let w = h.rect(window);
    let popup = h.open_menu(
        window,
        2,
        (w.x + 10, w.y + 10),
        menu(Rect::new(0, 0, 40, 20), 200, 150),
    );
    h.clear_log();
    let o = h.rect(other);
    h.scene.touch_down(0, o.x + 40, o.y + 60);
    h.scene.touch_up(0);
    h.pump();
    assert_eq!(h.dismissed(popup), Some(DismissReason::OutsidePress));
    assert!(h
        .events_for(OTHER)
        .iter()
        .all(|e| !matches!(e, Event::Touch { .. })));
    // Two fingers on the window, then the gesture is cancelled.
    h.scene.touch_down(1, w.x + 5, w.y + 5);
    h.scene.touch_down(2, w.x + 50, w.y + 50);
    h.scene.touch_cancel();
    h.pump();
    let cancels = h
        .events_for(APP)
        .iter()
        .filter(|e| {
            matches!(
                e,
                Event::Touch {
                    kind: TouchKind::Cancel,
                    ..
                }
            )
        })
        .count();
    assert_eq!(cancels, 2);
    assert_eq!(h.scene.stats().touch_points, 0);
}

#[test]
fn losing_the_captured_surface_never_turns_the_release_into_a_click() {
    let mut h = Harness::new();
    let other = h.toplevel(OTHER, 1, 1920, 1080);
    let window = h.toplevel(APP, 1, 400, 300);
    let w = h.rect(window);
    h.press(w.x + 10, w.y + 10);
    // The pressed window closes while the button is held.
    h.request(APP, Request::Destroy { surface: 1 }).unwrap();
    h.clear_log();
    h.scene.pointer_button(BUTTON, false);
    h.pump();
    assert_eq!(pointer_presses(&h.events_for(OTHER)), 0);
    // Explicit capture loss sends Cancel.
    h.press(100, 100);
    h.clear_log();
    h.scene.pointer_cancel();
    h.scene.pointer_button(BUTTON, false);
    h.pump();
    let events = h.events_for(OTHER);
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Pointer {
            kind: PointerKind::Cancel,
            ..
        }
    )));
    assert_eq!(pointer_presses(&events), 0);
    let _ = other;
}

#[test]
fn modal_dialogs_block_their_window_and_take_focus() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 800, 600);
    h.request(
        APP,
        Request::CreateDialog {
            surface: 2,
            parent: 1,
            size: Size::new(300, 200),
        },
    )
    .unwrap();
    h.present(key(APP, 2));
    assert_eq!(h.scene.focus(), Some(key(APP, 2)));
    let w = h.rect(window);
    h.clear_log();
    h.click(w.x + 5, w.y + 5);
    assert_eq!(pointer_presses(&h.events_for(APP)), 0);
    assert_eq!(h.scene.focus(), Some(key(APP, 2)));
    h.request(APP, Request::Destroy { surface: 2 }).unwrap();
    assert_eq!(h.scene.focus(), Some(window));
}

#[test]
fn output_scale_change_reconfigures_and_output_removal_dismisses_popups() {
    let mut h = Harness::new();
    h.scene
        .add_output(Output {
            id: 2,
            rect: Rect::new(1920, 0, 1280, 720),
            scale_120: 120,
            refresh_mhz: 120_000,
        })
        .unwrap();
    let window = h.toplevel(APP, 1, 400, 300);
    let w = h.rect(window);
    let popup = h.open_menu(
        window,
        2,
        (w.x + 10, w.y + 10),
        menu(Rect::new(0, 0, 40, 20), 200, 150),
    );
    h.scene.set_output_scale(1, 240).unwrap();
    h.pump();
    let (_, rect, scale) = h.configures[&window];
    assert_eq!((rect.width, rect.height, scale), (400, 300, 240));
    assert_eq!(
        h.rect(window),
        w,
        "logical geometry is unchanged; old content stays"
    );
    // The new buffer is twice as dense.
    h.attach(APP, 500, Size::new(800, 600));
    h.request(
        APP,
        Request::Submit {
            surface: 1,
            sequence: 10,
            buffer: 500,
            configure: h.configures[&window].0,
            extent: Size::new(800, 600),
        },
    )
    .unwrap();
    h.next_sequence.insert(window, 10);
    assert_eq!(h.dismissed(popup), None);
    h.scene.remove_output(1);
    h.pump();
    assert_eq!(h.dismissed(popup), Some(DismissReason::OutputRemoved));
    let (_, _, scale) = h.configures[&window];
    assert_eq!(scale, 120, "moved to the remaining output");
    h.present(window);
    let moved = h.rect(window);
    assert!(Rect::new(1920, 0, 1280, 720).contains(moved), "{moved:?}");
}

#[test]
fn revoking_a_grant_dismisses_privileged_surfaces() {
    let mut h = Harness::new();
    h.request(
        SHELL,
        Request::CreateShellSurface {
            surface: 1,
            kind: ShellKind::Overlay,
            edge: PanelEdge::None,
            output: 1,
            size: Size::new(600, 400),
            exclusive: 0,
        },
    )
    .unwrap();
    h.present(key(SHELL, 1));
    h.scene.revoke(
        SHELL,
        Grants {
            shell: true,
            ..Grants::default()
        },
    );
    h.pump();
    assert_eq!(
        h.dismissed(key(SHELL, 1)),
        Some(DismissReason::AuthorizationRevoked)
    );
    assert!(!h.rendered(key(SHELL, 1)));
    assert_eq!(
        h.request(
            SHELL,
            Request::CreateShellSurface {
                surface: 2,
                kind: ShellKind::Overlay,
                edge: PanelEdge::None,
                output: 1,
                size: Size::new(600, 400),
                exclusive: 0,
            }
        ),
        Err(ErrorCode::Unauthorized)
    );
}

impl Harness {
    fn activate(&mut self, surface: SurfaceKey) {
        self.scene.activate(surface);
        self.pump();
    }
}

#[test]
fn shell_moves_and_resizes_carry_an_open_menu() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let w = h.rect(window);
    let popup = h.open_menu(
        window,
        2,
        (w.x + 20, w.y + 20),
        menu(Rect::new(0, 280, 40, 20), 200, 150),
    );
    let before = h.rect(popup);
    h.clear_log();

    // A move: at once, the menu carried in the same frame and still open, nothing sent to the
    // client.
    h.scene.move_window_to(window, (w.x + 100, w.y + 50));
    let moved = h.rect(window);
    assert_eq!((moved.x, moved.y), (w.x + 100, w.y + 50));
    let carried = h.rect(popup);
    assert_eq!((carried.x - before.x, carried.y - before.y), (100, 50));
    assert_eq!(h.dismissed(popup), None);
    assert_eq!(h.scene.grab_chain(), &[popup]);
    assert!(h.events_for(APP).is_empty(), "{:?}", h.events_for(APP));

    // Kept on screen like an interactive move.
    h.scene.move_window_to(window, (100_000, 100_000));
    assert!(h.rect(window).x < SCREEN.right() as i32);
    h.scene.move_window_to(window, (w.x, w.y));

    // A resize: configured now, applied when presented; the menu stays open meanwhile.
    h.scene.resize_window(window, Size::new(600, 500));
    h.pump();
    assert_eq!(
        h.rect(window).width,
        400,
        "the old content stays until the new frame"
    );
    let (_, rect, _) = h.configures[&window];
    assert_eq!((rect.width, rect.height), (600, 500));
    h.present(window);
    assert_eq!((h.rect(window).width, h.rect(window).height), (600, 500));
    assert_eq!(h.dismissed(popup), None);

    // Never to an empty size.
    h.scene.resize_window(window, Size::new(0, 10));
    h.pump();
    assert_eq!(h.configures[&window].1.width, 600);
}

#[test]
fn parent_resize_reconstrains_popups_once_the_new_size_is_presented() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    h.request(
        APP,
        Request::SetFullscreen {
            surface: 1,
            enabled: true,
        },
    )
    .unwrap();
    h.present(window);
    // Near the bottom-right of the fullscreen window, so the menu flips up and left.
    let popup = h.popup(window, 2, menu(Rect::new(1800, 950, 20, 20), 200, 150));
    h.present(popup);
    let before = h.configures[&popup];
    assert_eq!((before.1.x, before.1.y), (1620, 800));
    h.request(
        APP,
        Request::SetFullscreen {
            surface: 1,
            enabled: false,
        },
    )
    .unwrap();
    h.frame();
    assert_eq!(
        h.configures[&popup], before,
        "nothing changes until the window's new frame"
    );
    h.present(window);
    let (serial, _, _) = h.configures[&popup];
    assert!(
        serial > before.0,
        "the popup is reconfigured for the smaller window"
    );
    h.present(popup);
    assert!(SCREEN.contains(h.rect(popup)), "{:?}", h.rect(popup));
}

#[test]
fn configures_only_representable_buffers_at_high_scale() {
    let mut h = Harness::new();
    h.scene.set_output_scale(1, 480).unwrap();
    h.request(
        APP,
        Request::CreateToplevel {
            surface: 1,
            size: Size::new(5000, 1000),
        },
    )
    .unwrap();
    let (_, rect, scale) = h.configures[&key(APP, 1)];
    // 4096 logical × 4 = 16384 physical: the largest buffer a client can submit.
    assert_eq!((rect.width, rect.height, scale), (4096, 1000, 480));
    h.present(key(APP, 1));
    assert_eq!(h.rect(key(APP, 1)).width, 4096);
}

#[test]
fn moving_a_window_to_a_denser_output_reconfigures_it_and_its_popups() {
    let mut h = Harness::new();
    h.scene
        .add_output(Output {
            id: 2,
            rect: Rect::new(1920, 0, 1920, 1080),
            scale_120: 240,
            refresh_mhz: 60_000,
        })
        .unwrap();
    let window = h.toplevel(APP, 1, 400, 300);
    let popup = h.popup(window, 2, menu(Rect::new(0, 280, 40, 20), 200, 150));
    h.present(popup);
    let w = h.rect(window);
    let serial = h.press(w.x + 200, w.y + 10);
    h.request(APP, Request::BeginMove { surface: 1, serial })
        .unwrap();
    h.scene.pointer_motion(w.x + 200 + 1500, w.y + 10);
    h.pump();
    assert_eq!(h.configures[&window].2, 240, "rescaled while moving");
    assert_eq!(
        h.rect(window).x,
        w.x + 1500,
        "old content keeps moving with the pointer"
    );
    h.scene.pointer_button(BUTTON, false);
    h.pump();
    assert_eq!(
        h.configures[&popup].2, 240,
        "the popup follows its new anchor output"
    );
    // The new buffers are twice as dense.
    h.present(window);
    h.present(popup);
    assert_eq!(h.rect(window).width, 400);
}

#[test]
fn entering_fullscreen_re_places_popups_against_the_new_geometry() {
    let mut h = Harness::new();
    // 1800 wide, centered at x=60: a menu at local x=1700 does not fit to the right and flips.
    let window = h.toplevel(APP, 1, 1800, 600);
    let popup = h.popup(window, 2, menu(Rect::new(1700, 100, 20, 20), 200, 150));
    h.present(popup);
    let before = h.configures[&popup];
    assert_eq!(before.1.x, 1520, "flipped left of the anchor");
    h.request(
        APP,
        Request::SetFullscreen {
            surface: 1,
            enabled: true,
        },
    )
    .unwrap();
    h.frame();
    assert_eq!(
        h.configures[&popup], before,
        "unchanged until the fullscreen frame shows"
    );
    h.present(window);
    let (serial, rect, _) = h.configures[&popup];
    assert!(serial > before.0);
    assert_eq!(rect.x, 1700, "now fits on its preferred side");
    h.present(popup);
    assert!(SCREEN.contains(h.rect(popup)));
}

#[test]
fn a_fullscreen_window_fills_the_surviving_output_after_removal() {
    let mut h = Harness::new();
    let small = Rect::new(1920, 0, 1280, 720);
    h.scene
        .add_output(Output {
            id: 2,
            rect: small,
            scale_120: 120,
            refresh_mhz: 60_000,
        })
        .unwrap();
    let window = h.toplevel(APP, 1, 800, 600);
    h.request(
        APP,
        Request::SetFullscreen {
            surface: 1,
            enabled: true,
        },
    )
    .unwrap();
    h.present(window);
    h.scene.remove_output(1);
    h.pump();
    let (_, rect, _) = h.configures[&window];
    assert_eq!((rect.width, rect.height), (1280, 720));
    h.present(window);
    assert_eq!(h.rect(window), small);
    // Leaving fullscreen restores the windowed size on the surviving output.
    h.request(
        APP,
        Request::SetFullscreen {
            surface: 1,
            enabled: false,
        },
    )
    .unwrap();
    h.present(window);
    let restored = h.rect(window);
    assert_eq!((restored.width, restored.height), (800, 600));
    assert!(small.contains(restored), "{restored:?}");
}

#[test]
fn shortcuts_move_to_another_shell_surface_when_the_target_goes_away() {
    let mut h = Harness::new();
    for (surface, kind) in [(1, ShellKind::Panel), (2, ShellKind::Overlay)] {
        h.request(
            SHELL,
            Request::CreateShellSurface {
                surface,
                kind,
                edge: PanelEdge::None,
                output: 1,
                size: Size::new(100, 40),
                exclusive: 0,
            },
        )
        .unwrap();
    }
    h.toplevel(APP, 1, 400, 300);
    h.request(SHELL, Request::Destroy { surface: 1 }).unwrap();
    h.clear_log();
    h.scene.key(SUPER, true);
    h.pump();
    assert!(h.events_for(SHELL).contains(&Event::Key {
        surface: 2,
        serial: h.scene.serial,
        key: SUPER,
        pressed: true
    }));
    assert!(h.events_for(APP).is_empty());
}

#[test]
fn a_finished_move_leaves_no_pointer_capture_behind() {
    let mut h = Harness::new();
    let other = h.toplevel(OTHER, 1, 1200, 800);
    let window = h.toplevel(APP, 1, 300, 200);
    let w = h.rect(window);
    let serial = h.press(w.x + 100, w.y + 10);
    h.request(APP, Request::BeginMove { surface: 1, serial })
        .unwrap();
    h.scene.pointer_motion(w.x + 300, w.y + 10);
    h.scene.pointer_button(BUTTON, false);
    h.clear_log();
    let o = h.rect(other);
    h.click(o.x + 20, o.y + 20);
    assert_eq!(
        pointer_presses(&h.events_for(OTHER)),
        2,
        "the next click is OTHER's"
    );
    assert!(h.events_for(APP).iter().all(|e| !matches!(
        e,
        Event::Pointer {
            kind: PointerKind::Press | PointerKind::Motion,
            ..
        }
    )));
}

#[test]
fn a_consumed_press_invalidates_older_grab_serials() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let w = h.rect(window);
    let serial = h.press(w.x + 10, w.y + 10);
    let first = h.popup(window, 2, menu(Rect::new(0, 0, 40, 20), 200, 150));
    h.request(APP, Request::GrabPopup { surface: 2, serial })
        .unwrap();
    h.present(first);
    h.scene.pointer_button(BUTTON, false);
    h.click(5, 5);
    assert_eq!(h.dismissed(first), Some(DismissReason::OutsidePress));
    // Re-grabbing with the trigger's serial after the dismissing press is denied.
    let again = h.popup(window, 3, menu(Rect::new(0, 0, 40, 20), 200, 150));
    h.request(APP, Request::GrabPopup { surface: 3, serial })
        .unwrap();
    assert_eq!(h.dismissed(again), Some(DismissReason::GrabDenied));
    assert!(h.scene.grab_chain().is_empty());
}

#[test]
fn releases_stay_balanced_after_an_outside_press_with_buttons_held() {
    let mut h = Harness::new();
    let other = h.toplevel(OTHER, 1, 1200, 800);
    let window = h.toplevel(APP, 1, 300, 200);
    let w = h.rect(window);
    let popup = h.open_menu(
        window,
        2,
        (w.x + 10, w.y + 10),
        menu(Rect::new(0, 0, 40, 20), 200, 150),
    );
    let p = h.rect(popup);
    h.scene.pointer_motion(p.x + 10, p.y + 10);
    h.scene.pointer_button(BUTTON, true);
    let o = h.rect(other);
    h.scene.pointer_motion(o.x + 20, o.y + 20);
    h.scene.pointer_button(BUTTON + 1, true);
    h.scene.pointer_button(BUTTON + 1, false);
    h.scene.pointer_button(BUTTON, false);
    h.clear_log();
    h.click(o.x + 20, o.y + 20);
    assert_eq!(
        pointer_presses(&h.events_for(OTHER)),
        2,
        "press and release both delivered"
    );
}

#[test]
fn a_buffer_awaiting_release_cannot_be_queued_again() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let (serial, _, _) = h.configures[&window];
    let size = Size::new(400, 300);
    for buffer in [500, 501] {
        h.attach(APP, buffer, size);
    }
    h.scene.begin_frame();
    let submit = |sequence, buffer| Request::Submit {
        surface: 1,
        sequence,
        buffer,
        configure: serial,
        extent: size,
    };
    h.request(APP, submit(10, 500)).unwrap();
    h.request(APP, submit(11, 501)).unwrap();
    assert_eq!(
        h.request(APP, submit(12, 500)),
        Err(ErrorCode::InvalidBuffer)
    );
}

#[test]
fn a_window_mapped_after_the_user_typed_elsewhere_does_not_take_focus() {
    let mut h = Harness::new();
    let other = h.toplevel(OTHER, 1, 800, 600);
    assert_eq!(h.scene.focus(), Some(other));
    h.scene.key(30, true);
    h.scene.key(30, false);
    let window = h.toplevel(APP, 1, 300, 200);
    assert_eq!(
        h.scene.focus(),
        Some(other),
        "keystrokes keep going to OTHER"
    );
    let order: Vec<SurfaceKey> = h
        .scene
        .render_list()
        .into_iter()
        .filter_map(|item| match item {
            RenderItem::Surface { key, .. } => Some(key),
            _ => None,
        })
        .collect();
    assert_eq!(
        order,
        vec![window, other],
        "mapped behind the focused window"
    );
    // Once the user activates it, it behaves normally.
    let w = h.rect(window);
    let o = h.rect(other);
    assert!(o.contains(w));
    h.scene.activate(window);
    assert_eq!(h.scene.focus(), Some(window));
}

#[test]
fn unencodable_popup_offsets_are_unplaceable() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let mut far = menu(Rect::new(10, 10, 20, 20), 100, 100);
    far.adjust = Adjust::NONE;
    far.offset_x = oxyos_protocol::MAX_COORDINATE;
    let popup = h.popup(window, 2, far);
    assert_eq!(h.dismissed(popup), Some(DismissReason::Unplaceable));
    // Every event the scene produced can be encoded.
    for (_, event) in &h.log {
        let mut bytes = Vec::new();
        oxyos_protocol::write_event(&mut bytes, event).unwrap();
    }
}

#[test]
fn a_lock_surface_never_shows_through_another_outputs_cover() {
    let mut h = Harness::new();
    // A cloned output overlapping the first one exactly.
    h.scene
        .add_output(Output {
            id: 2,
            rect: SCREEN,
            scale_120: 120,
            refresh_mhz: 60_000,
        })
        .unwrap();
    h.request(
        LOCKER,
        Request::CreateLockSurface {
            surface: 1,
            output: 1,
        },
    )
    .unwrap();
    h.present(key(LOCKER, 1));
    h.scene.lock();
    let list = h.scene.render_list();
    assert_eq!(
        list.last(),
        Some(&RenderItem::Cover {
            output: 2,
            rect: SCREEN
        }),
        "output 2's cover is on top of output 1's lock surface"
    );
}

#[test]
fn buffers_are_owned_per_connection_and_retired_only_when_unused() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let (serial, _, _) = h.configures[&window];
    let submit = |buffer, sequence, extent| Request::Submit {
        surface: 1,
        sequence,
        buffer,
        configure: serial,
        extent,
    };
    let size = Size::new(400, 300);
    // Unregistered buffer ids, including another client's, are invalid.
    assert_eq!(
        h.request(APP, submit(777, 10, size)),
        Err(ErrorCode::InvalidBuffer)
    );
    h.attach(APP, 50, size);
    let other = h.toplevel(OTHER, 1, 400, 300);
    let other_serial = h.configures[&other].0;
    assert_eq!(
        h.request(
            OTHER,
            Request::Submit {
                surface: 1,
                sequence: 10,
                buffer: 50,
                configure: other_serial,
                extent: size
            }
        ),
        Err(ErrorCode::InvalidBuffer),
        "APP's buffer 50 does not exist for OTHER"
    );
    // Layout and duplicate checks.
    let attach = |buffer, stride| Request::AttachBuffer {
        buffer,
        format: oxyos_protocol::Format::Argb8888,
        extent: size,
        stride,
        offset: 0,
    };
    assert_eq!(
        h.request(APP, attach(51, 1599)),
        Err(ErrorCode::InvalidBuffer)
    );
    assert_eq!(
        h.request(APP, attach(50, 1600)),
        Err(ErrorCode::InvalidBuffer)
    );
    // The buffer's extent must match the submission.
    h.attach(APP, 52, Size::new(10, 10));
    assert_eq!(
        h.request(APP, submit(52, 10, size)),
        Err(ErrorCode::InvalidBuffer)
    );
    // A buffer cannot back two surfaces at once.
    h.request(APP, submit(50, 10, size)).unwrap();
    h.frame();
    let popup = h.popup(window, 2, menu(Rect::new(0, 0, 10, 10), 400, 300));
    let popup_serial = h.configures[&popup].0;
    assert_eq!(
        h.request(
            APP,
            Request::Submit {
                surface: 2,
                sequence: 1,
                buffer: 50,
                configure: popup_serial,
                extent: size
            }
        ),
        Err(ErrorCode::InvalidBuffer)
    );
    // Destroying a displayed buffer defers retirement until it is replaced.
    h.scene.drain_retired();
    h.request(APP, Request::DestroyBuffer { buffer: 50 })
        .unwrap();
    assert!(h.scene.drain_retired().is_empty(), "still on screen");
    assert_eq!(
        h.request(APP, submit(50, 11, size)),
        Err(ErrorCode::InvalidBuffer)
    );
    assert_eq!(
        h.request(APP, Request::DestroyBuffer { buffer: 50 }),
        Err(ErrorCode::InvalidBuffer)
    );
    h.next_sequence.insert(window, 20);
    h.present(window);
    assert_eq!(h.scene.drain_retired(), vec![(APP, 50)]);
    // An unused buffer is retired immediately.
    h.request(APP, Request::DestroyBuffer { buffer: 52 })
        .unwrap();
    assert_eq!(h.scene.drain_retired(), vec![(APP, 52)]);
}

#[test]
fn buffer_registrations_are_bounded_per_client() {
    let mut h = Harness::new();
    for buffer in 1..=MAX_BUFFERS_PER_CLIENT as u64 {
        h.attach(OTHER, buffer, Size::new(1, 1));
    }
    assert_eq!(
        h.request(
            OTHER,
            Request::AttachBuffer {
                buffer: 9999,
                format: oxyos_protocol::Format::Xrgb8888,
                extent: Size::new(1, 1),
                stride: 4,
                offset: 0
            }
        ),
        Err(ErrorCode::QuotaExceeded)
    );
    h.toplevel(APP, 1, 10, 10);
    h.scene.disconnect(OTHER);
    assert_eq!(h.scene.drain_retired().len(), MAX_BUFFERS_PER_CLIENT);
    assert_eq!(h.scene.stats().registered_buffers, 1);
}

#[test]
fn the_frame_list_shows_in_flight_content_with_its_own_geometry() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let shown = h.rect(window);
    h.request(
        APP,
        Request::SetFullscreen {
            surface: 1,
            enabled: true,
        },
    )
    .unwrap();
    let buffer = h.submit(window);
    let popup = h.popup(window, 2, menu(Rect::new(0, 0, 10, 10), 50, 50));
    h.submit(popup);
    h.scene.begin_frame();
    let frame: Vec<(SurfaceKey, Rect, u64)> = h
        .scene
        .frame_list()
        .into_iter()
        .filter_map(|item| match item {
            RenderItem::Surface { key, rect, frame } => Some((key, rect, frame.buffer)),
            _ => None,
        })
        .collect();
    assert_eq!(
        frame[0],
        (window, SCREEN, buffer),
        "new size and buffer together"
    );
    assert_eq!(
        frame[1].0, popup,
        "a first frame is composed in the frame that maps it"
    );
    assert_eq!(
        h.rect(window),
        shown,
        "displayed state is unchanged until completion"
    );
    assert!(!h.rendered(popup));
    h.scene.end_frame(1, Clock::Synthetic);
    assert_eq!(h.rect(window), SCREEN);
    assert!(h.rendered(popup));
}

#[test]
fn an_output_attached_while_locked_is_covered_immediately() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    h.request(
        LOCKER,
        Request::CreateLockSurface {
            surface: 1,
            output: 1,
        },
    )
    .unwrap();
    h.present(key(LOCKER, 1));
    h.scene.lock();
    let second = Rect::new(1920, 0, 1280, 720);
    h.scene
        .add_output(Output {
            id: 2,
            rect: second,
            scale_120: 120,
            refresh_mhz: 60_000,
        })
        .unwrap();
    // The new output shows a compositor cover, never application content, before any lock UI
    // exists for it.
    let list = h.scene.render_list();
    assert!(list.contains(&RenderItem::Cover {
        output: 2,
        rect: second
    }));
    assert!(!h.rendered(window));
    // Moving the pointer onto it and pressing delivers nothing to applications.
    h.clear_log();
    h.click(2000, 100);
    assert!(h.events_for(APP).is_empty());
    // A lock surface for the new output replaces the cover once it has content.
    h.request(
        LOCKER,
        Request::CreateLockSurface {
            surface: 2,
            output: 2,
        },
    )
    .unwrap();
    assert!(h.scene.render_list().contains(&RenderItem::Cover {
        output: 2,
        rect: second
    }));
    h.present(key(LOCKER, 2));
    assert!(!h
        .scene
        .render_list()
        .iter()
        .any(|item| matches!(item, RenderItem::Cover { .. })));
}

#[test]
fn an_in_flight_buffer_of_a_destroyed_surface_cannot_back_another_surface() {
    let mut h = Harness::new();
    let first = h.toplevel(APP, 1, 400, 300);
    let buffer = h.submit(first);
    h.request(
        APP,
        Request::CreateToplevel {
            surface: 2,
            size: Size::new(400, 300),
        },
    )
    .unwrap();
    h.scene.begin_frame();
    h.request(APP, Request::Destroy { surface: 1 }).unwrap();
    let (serial, _, _) = h.configures[&key(APP, 2)];
    assert_eq!(
        h.request(
            APP,
            Request::Submit {
                surface: 2,
                sequence: 1,
                buffer,
                configure: serial,
                extent: Size::new(400, 300)
            }
        ),
        Err(ErrorCode::InvalidBuffer),
        "the composition may still read it"
    );
    h.scene.end_frame(2, Clock::Synthetic);
    h.pump();
    assert!(h
        .events_for(APP)
        .contains(&Event::Released { surface: 1, buffer }));
}

#[test]
fn a_pinch_goes_to_the_surface_under_the_pointer_when_it_began_until_it_ends() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let other = h.toplevel(OTHER, 1, 200, 100);
    let w = h.rect(window);
    h.scene.pointer_motion(w.x + 10, w.y + 20);
    h.pump();
    h.clear_log();
    let focus = h.scene.focus();
    let pinches = |events: Vec<(ClientId, Event)>| -> Vec<_> {
        events
            .into_iter()
            .filter_map(|(client, e)| match e {
                Event::Pinch {
                    surface,
                    phase,
                    x,
                    y,
                    scale_65536,
                    rotation_256,
                    ..
                } => Some((client, surface, phase, x, y, scale_65536, rotation_256)),
                _ => None,
            })
            .collect()
    };
    // An update with no pinch begun reaches nobody.
    h.scene.pointer_pinch(GesturePhase::Update, 65536, 0);
    assert!(pinches(h.pump()).is_empty());
    // At the pointer, in the window's coordinates (1/256 of a pixel, as a pointer event's).
    h.scene.pointer_pinch(GesturePhase::Begin, 65536, 0);
    h.scene.pointer_pinch(GesturePhase::Update, 98304, -2560);
    assert_eq!(
        pinches(h.pump()),
        [
            (APP, 1, GesturePhase::Begin, 10 * 256, 20 * 256, 65536, 0),
            (
                APP,
                1,
                GesturePhase::Update,
                10 * 256,
                20 * 256,
                98304,
                -2560
            ),
        ]
    );
    // The window keeps the pinch until it ends, with the pointer over the other window.
    let o = h.rect(other);
    h.scene.pointer_motion(o.x + 10, o.y + 10);
    h.pump();
    h.scene.pointer_pinch(GesturePhase::Update, 131072, 0);
    h.scene.pointer_pinch(GesturePhase::End, 131072, 0);
    let (x, y) = ((o.x + 10 - w.x) * 256, (o.y + 10 - w.y) * 256);
    assert_eq!(
        pinches(h.pump()),
        [
            (APP, 1, GesturePhase::Update, x, y, 131072, 0),
            (APP, 1, GesturePhase::End, x, y, 131072, 0),
        ]
    );
    // After it ends, an update without a new beginning reaches nobody.
    h.scene.pointer_pinch(GesturePhase::Update, 65536, 0);
    assert!(pinches(h.pump()).is_empty());
    // A pinch is not a press: focus and stacking are unchanged.
    assert_eq!(h.scene.focus(), focus);
}

#[test]
fn scrolls_go_to_the_surface_under_the_pointer_without_activating_it() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    // Both are centered; the smaller one, on top, covers only the middle of the window.
    let other = h.toplevel(OTHER, 1, 200, 100);
    let w = h.rect(window);
    h.scene.pointer_motion(w.x + 10, w.y + 20);
    h.pump();
    h.clear_log();
    let focus = h.scene.focus();
    h.scene.pointer_scroll(0, -256 * 40);
    let events = h.pump();
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        (
            APP,
            Event::Scroll {
                surface: 1,
                dx: 0,
                dy: -10240,
                ..
            }
        )
    ));
    // Scrolling is not a press: focus and stacking are unchanged.
    assert_eq!(h.scene.focus(), focus);

    // Over a popup grab, scrolls outside the grab reach nobody.
    let menu_popup = h.open_menu(
        window,
        2,
        (w.x + 10, w.y + 10),
        menu(Rect::new(0, 0, 4, 4), 50, 50),
    );
    let o = h.rect(other);
    h.scene.pointer_motion(o.x + 10, o.y + 10);
    h.pump();
    h.scene.pointer_scroll(0, 256);
    assert!(!h
        .pump()
        .iter()
        .any(|(_, e)| matches!(e, Event::Scroll { .. })));
    assert_eq!(h.scene.grab_chain(), &[menu_popup]);
}

#[test]
fn text_goes_to_the_focused_surface_in_whole_character_chunks() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    h.pump();
    let text = format!("{}é!", "a".repeat(31));
    h.scene.text(&text);
    let chunks: Vec<String> = h
        .pump()
        .into_iter()
        .filter_map(|(client, event)| match event {
            Event::Text {
                surface: 1, text, ..
            } if client == APP => Some(text.as_str().to_string()),
            _ => None,
        })
        .collect();
    // 31 bytes, then the two-byte character does not fit the 32-byte chunk.
    assert_eq!(chunks, [&text[..31], &text[31..]]);
    assert_eq!(chunks.concat(), text);
}

#[test]
fn preedit_goes_to_the_focused_surface_and_rejects_what_does_not_fit() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    h.pump();
    assert!(h.scene.preedit("にほ", 3));
    let delivered: Vec<(String, u8)> = h
        .pump()
        .into_iter()
        .filter_map(|(_, event)| match event {
            Event::Preedit {
                surface: 1,
                text,
                cursor,
                ..
            } => Some((text.as_str().to_string(), cursor)),
            _ => None,
        })
        .collect();
    assert_eq!(delivered, [("にほ".to_string(), 3)]);
    // An empty preedit ends the composition.
    assert!(h.scene.preedit("", 0));
    assert!(h
        .pump()
        .iter()
        .any(|(_, e)| matches!(e, Event::Preedit { cursor: 0, .. })));
    // A cursor past the end or inside a character: nothing is delivered.
    assert!(!h.scene.preedit("ab", 3));
    assert!(!h.scene.preedit("に", 1));
    assert!(h.pump().is_empty());
    // A composition longer than one Preedit cannot be shown without text input: the field is
    // told the composition ended rather than shown part of it.
    assert!(h.scene.preedit(&"a".repeat(33), 33));
    let ended: Vec<Event> = h.pump().into_iter().map(|(_, event)| event).collect();
    assert!(matches!(
        ended.as_slice(),
        [Event::Preedit { text, cursor: 0, .. }] if text.as_str().is_empty()
    ));
}

// ---- text input (ADR 0008) -------------------------------------------------------------------

/// Enables text input on `surface` with `surrounding` and the caret at its end.
fn enable_text_input(h: &mut Harness, surface: SurfaceKey, surrounding: &str, serial: u32) {
    let id = surface.id;
    h.request(surface.client, Request::TextInputEnable { surface: id })
        .unwrap();
    for text in TextChunk::split(surrounding) {
        h.request(
            surface.client,
            Request::TextInputSurrounding { surface: id, text },
        )
        .unwrap();
    }
    let end = surrounding.len() as u32;
    h.request(
        surface.client,
        Request::TextInputSelection {
            surface: id,
            anchor: end,
            cursor: end,
        },
    )
    .unwrap();
    h.request(
        surface.client,
        Request::TextInputCommit {
            surface: id,
            serial,
        },
    )
    .unwrap();
}

/// A transaction as delivered: (deletion, committed text, composition, cursor, segments, Done's
/// input serial and commit serial), or `None` if the events are not exactly one transaction.
type Delivered = (
    Option<(u32, u32)>,
    String,
    String,
    Option<(u32, u32)>,
    Vec<(u32, u32, PreeditStyle)>,
    u64,
    u32,
);

fn transaction(events: &[Event]) -> Option<Delivered> {
    let mut delivered: Delivered = (None, String::new(), String::new(), None, Vec::new(), 0, 0);
    let (last, rest) = events.split_last()?;
    let Event::TextInputDone { serial, commit, .. } = *last else {
        return None;
    };
    (delivered.5, delivered.6) = (serial, commit);
    // Each part at most once, in the protocol's order.
    let mut stage = 0;
    for event in rest {
        let next = match *event {
            Event::TextInputDelete { before, after, .. } if stage < 1 => {
                delivered.0 = Some((before, after));
                1
            }
            Event::TextInputCommitText { text, .. } if stage <= 2 => {
                delivered.1.push_str(text.as_str());
                2
            }
            Event::TextInputPreeditText { text, .. } if stage <= 3 => {
                delivered.2.push_str(text.as_str());
                3
            }
            Event::TextInputPreeditCursor { begin, end, .. } if stage == 3 => {
                delivered.3 = Some((begin, end));
                4
            }
            Event::TextInputPreeditSegment {
                begin, end, style, ..
            } if stage >= 4 => {
                delivered.4.push((begin, end, style));
                5
            }
            _ => return None,
        };
        stage = next;
    }
    Some(delivered)
}

#[test]
fn a_composition_longer_than_a_chunk_arrives_whole_as_one_transaction() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    enable_text_input(&mut h, window, "", 1);
    h.pump();
    // 40 bytes of Japanese: more than one 32-byte chunk.
    let composition = "にほんごのにゅうりょくですね";
    assert!(composition.len() > 32);
    assert!(h.scene.preedit(composition, composition.len()));
    let events: Vec<Event> = h.pump().into_iter().map(|(_, event)| event).collect();
    // No Preedit for a field with text input: only the transaction.
    assert!(!events.iter().any(|e| matches!(e, Event::Preedit { .. })));
    let (deletion, commit, preedit, cursor, segments, _, serial) =
        transaction(&events).expect("one transaction");
    assert_eq!(deletion, None);
    assert_eq!(commit, "");
    assert_eq!(preedit, composition);
    let end = composition.len() as u32;
    assert_eq!(cursor, Some((end, end)));
    assert_eq!(segments, [(0, end, PreeditStyle::Composing)]);
    assert_eq!(serial, 1);
    // Committed text ends the composition in the same transaction.
    h.scene.text("日本語");
    let events: Vec<Event> = h.pump().into_iter().map(|(_, event)| event).collect();
    assert!(!events.iter().any(|e| matches!(e, Event::Text { .. })));
    let (_, commit, preedit, cursor, ..) = transaction(&events).expect("one transaction");
    assert_eq!(
        (commit.as_str(), preedit.as_str(), cursor),
        ("日本語", "", None)
    );
}

#[test]
fn a_deletion_commit_and_composition_arrive_as_one_transaction() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    enable_text_input(&mut h, window, "にほ", 4);
    h.pump();
    let segments = [
        (0, 3, PreeditStyle::Composing),
        (3, 6, PreeditStyle::Converting),
    ];
    assert!(h.scene.text_transaction(&TextTransaction {
        delete_before: 6,
        delete_after: 0,
        commit: "日本",
        preedit: "ごが",
        cursor: (3, 6),
        segments: &segments,
    }));
    let events = h.pump();
    assert!(events.iter().all(|(client, _)| *client == APP));
    let events: Vec<Event> = events.into_iter().map(|(_, event)| event).collect();
    let (deletion, commit, preedit, cursor, delivered, input, serial) =
        transaction(&events).expect("one transaction");
    assert_eq!(deletion, Some((6, 0)));
    assert_eq!((commit.as_str(), preedit.as_str()), ("日本", "ごが"));
    assert_eq!(cursor, Some((3, 6)));
    assert_eq!(delivered, segments);
    // Done carries the field's commit, and its input serial authorizes as a key press does.
    assert_eq!(serial, 4);
    assert!(h.scene.recent_input(APP, input));
    // A composition alone is not user input.
    assert!(h.scene.preedit("ご", 3));
    let events: Vec<Event> = h.pump().into_iter().map(|(_, event)| event).collect();
    let (.., input, _) = transaction(&events).expect("one transaction");
    assert!(!h.scene.recent_input(APP, input));
}

#[test]
fn done_names_the_latest_commit_and_stale_serials_are_refused() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    enable_text_input(&mut h, window, "a", 7);
    let commit =
        |h: &mut Harness, serial| h.request(APP, Request::TextInputCommit { surface: 1, serial });
    assert_eq!(commit(&mut h, 7), Err(ErrorCode::InvalidOrder));
    assert_eq!(commit(&mut h, 3), Err(ErrorCode::InvalidOrder));
    assert_eq!(commit(&mut h, 9), Ok(()));
    h.pump();
    h.scene.text("b");
    let events: Vec<Event> = h.pump().into_iter().map(|(_, event)| event).collect();
    assert_eq!(transaction(&events).expect("one transaction").6, 9);
    // A commit that describes nothing new keeps what the field said before.
    let state = h.scene.text_input(window).expect("enabled");
    assert_eq!((state.surrounding.as_str(), state.cursor), ("a", 1));
}

#[test]
fn a_field_must_describe_itself_within_its_limits() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    let chunk = |text: &str| TextChunk::new(text).unwrap();
    assert_eq!(
        h.request(APP, Request::TextInputEnable { surface: 9 }),
        Err(ErrorCode::UnknownSurface)
    );
    assert_eq!(
        h.request(
            APP,
            Request::TextInputSurrounding {
                surface: 9,
                text: chunk("a"),
            }
        ),
        Err(ErrorCode::UnknownSurface)
    );
    // A selection past the text, or inside a character.
    for (surrounding, cursor, serial) in [("ab", 3, 1), ("に", 1, 2)] {
        h.request(APP, Request::TextInputEnable { surface: 1 })
            .unwrap();
        h.request(
            APP,
            Request::TextInputSurrounding {
                surface: 1,
                text: chunk(surrounding),
            },
        )
        .unwrap();
        h.request(
            APP,
            Request::TextInputSelection {
                surface: 1,
                anchor: 0,
                cursor,
            },
        )
        .unwrap();
        assert_eq!(
            h.request(APP, Request::TextInputCommit { surface: 1, serial }),
            Err(ErrorCode::InvalidMessage)
        );
    }
    // At most 4,000 bytes of surrounding text.
    let thirty_two = "a".repeat(32);
    for _ in 0..MAX_SURROUNDING_BYTES / 32 {
        h.request(
            APP,
            Request::TextInputSurrounding {
                surface: 1,
                text: chunk(&thirty_two),
            },
        )
        .unwrap();
    }
    let rest = "a".repeat(MAX_SURROUNDING_BYTES % 32);
    if !rest.is_empty() {
        h.request(
            APP,
            Request::TextInputSurrounding {
                surface: 1,
                text: chunk(&rest),
            },
        )
        .unwrap();
    }
    assert_eq!(
        h.request(
            APP,
            Request::TextInputSurrounding {
                surface: 1,
                text: chunk("a"),
            }
        ),
        Err(ErrorCode::QuotaExceeded)
    );
}

#[test]
fn surrounding_text_is_replaced_at_each_commit_that_sends_it() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    enable_text_input(&mut h, window, &"x".repeat(40), 1);
    enable_text_input(&mut h, window, "hola", 2);
    let state = h.scene.text_input(window).expect("enabled");
    assert_eq!(state.surrounding, "hola");
    // Enabling again describes a new field: what the old one said is gone.
    h.request(
        APP,
        Request::TextInputPurpose {
            surface: 1,
            purpose: TextPurpose::Email,
            hints: TextHints::default(),
        },
    )
    .unwrap();
    h.request(
        APP,
        Request::TextInputCaret {
            surface: 1,
            rect: Rect::new(10, 4, 2, 18),
        },
    )
    .unwrap();
    h.request(
        APP,
        Request::TextInputCommit {
            surface: 1,
            serial: 3,
        },
    )
    .unwrap();
    let state = h.scene.text_input(window).expect("enabled").clone();
    assert_eq!(state.purpose, TextPurpose::Email);
    assert_eq!(state.caret, Some(Rect::new(10, 4, 2, 18)));
    h.request(APP, Request::TextInputEnable { surface: 1 })
        .unwrap();
    h.request(
        APP,
        Request::TextInputCommit {
            surface: 1,
            serial: 4,
        },
    )
    .unwrap();
    let state = h.scene.text_input(window).expect("enabled");
    assert_eq!(state.purpose, TextPurpose::Normal);
    assert_eq!((state.surrounding.as_str(), state.caret), ("", None));
}

#[test]
fn a_field_without_committed_text_input_receives_the_older_events() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    // Enabled but not committed: not in effect yet.
    h.request(APP, Request::TextInputEnable { surface: 1 })
        .unwrap();
    h.pump();
    h.scene.text("a");
    assert!(matches!(
        h.pump().as_slice(),
        [(APP, Event::Text { surface: 1, .. })]
    ));
    h.request(
        APP,
        Request::TextInputCommit {
            surface: 1,
            serial: 1,
        },
    )
    .unwrap();
    h.scene.text("b");
    let events: Vec<Event> = h.pump().into_iter().map(|(_, event)| event).collect();
    assert!(transaction(&events).is_some());
    // Disabled at its next commit.
    h.request(APP, Request::TextInputDisable { surface: 1 })
        .unwrap();
    h.request(
        APP,
        Request::TextInputCommit {
            surface: 1,
            serial: 2,
        },
    )
    .unwrap();
    assert!(h.scene.text_input(window).is_none());
    h.scene.text("c");
    assert!(matches!(
        h.pump().as_slice(),
        [(APP, Event::Text { surface: 1, .. })]
    ));
    // A deletion cannot be said with the older events: only the commit arrives.
    assert!(h.scene.text_transaction(&TextTransaction {
        delete_before: 1,
        commit: "d",
        ..TextTransaction::default()
    }));
    let events = h.pump();
    assert_eq!(events.len(), 1);
    assert!(matches!(&events[0], (APP, Event::Text { text, .. }) if text.as_str() == "d"));
}

#[test]
fn only_the_focused_field_receives_changes() {
    let mut h = Harness::new();
    let first = h.toplevel(APP, 1, 400, 300);
    let second = h.toplevel(OTHER, 1, 400, 300);
    enable_text_input(&mut h, first, "", 1);
    enable_text_input(&mut h, second, "", 1);
    h.activate(first);
    h.pump();
    h.scene.text("a");
    let events = h.pump();
    assert!(!events.is_empty());
    assert!(events.iter().all(|(client, _)| *client == APP));
    h.activate(second);
    h.scene.text("b");
    let events = h.pump();
    assert!(!events.is_empty());
    assert!(events.iter().all(|(client, _)| *client == OTHER));
}

#[test]
fn an_invalid_change_delivers_nothing() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    enable_text_input(&mut h, window, "", 1);
    h.pump();
    let many = [(0, 1, PreeditStyle::Composing); MAX_PREEDIT_SEGMENTS + 1];
    let long = "a".repeat(MAX_PREEDIT_BYTES + 1);
    for change in [
        // A cursor inside a character, past the end, or backwards.
        TextTransaction {
            preedit: "に",
            cursor: (1, 1),
            ..TextTransaction::default()
        },
        TextTransaction {
            preedit: "ab",
            cursor: (0, 3),
            ..TextTransaction::default()
        },
        TextTransaction {
            preedit: "ab",
            cursor: (2, 1),
            ..TextTransaction::default()
        },
        // An empty segment, one past the end, too many.
        TextTransaction {
            preedit: "ab",
            segments: &[(1, 1, PreeditStyle::Converting)],
            ..TextTransaction::default()
        },
        TextTransaction {
            preedit: "ab",
            segments: &[(0, 3, PreeditStyle::Error)],
            ..TextTransaction::default()
        },
        TextTransaction {
            preedit: "ab",
            segments: &many,
            ..TextTransaction::default()
        },
        // Too long a composition, or a deletion beyond any surrounding text.
        TextTransaction {
            preedit: &long,
            ..TextTransaction::default()
        },
        TextTransaction {
            delete_after: MAX_SURROUNDING_BYTES as u32 + 1,
            ..TextTransaction::default()
        },
    ] {
        assert!(!h.scene.text_transaction(&change), "{change:?}");
    }
    assert!(h.pump().is_empty());
}

#[test]
fn a_dismissed_popup_may_still_describe_its_field() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let w = h.rect(window);
    let popup = h.open_menu(
        window,
        2,
        (w.x + 10, w.y + 10),
        menu(Rect::new(0, 0, 4, 4), 200, 100),
    );
    // A press outside dismisses it; the client has not heard yet.
    h.click(w.x + 380, w.y + 280);
    assert!(h.dismissed(popup).is_some());
    enable_text_input(&mut h, popup, "buscar", 1);
    h.pump();
    h.scene.text("x");
    assert!(!h
        .pump()
        .iter()
        .any(|(_, event)| matches!(event, Event::TextInputDone { surface: 2, .. })));
}

#[test]
fn a_destroyed_surface_forgets_its_text_input() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    enable_text_input(&mut h, window, "secret", 1);
    h.request(APP, Request::Destroy { surface: 1 }).unwrap();
    assert!(h.scene.text_inputs.is_empty());
    // A new surface with the same id starts with nothing, its serials included.
    let window = h.toplevel(APP, 1, 400, 300);
    assert!(h.scene.text_input(window).is_none());
    enable_text_input(&mut h, window, "", 1);
    h.scene.disconnect(APP);
    assert!(h.scene.text_inputs.is_empty());
}

// ---- accessibility (ADR 0003) ----------------------------------------------------------------

fn reader() -> Grants {
    Grants {
        accessibility: true,
        ..Grants::default()
    }
}

/// Publishes a menu-like snapshot on `surface`: a root and two children, one focused.
fn publish(h: &mut Harness, surface: SurfaceKey, label: &str) {
    let state = |bits| AccessibleState::new(bits).unwrap();
    h.request(
        surface.client,
        Request::AccessibilityBegin {
            surface: surface.id,
            nodes: 3,
        },
    )
    .unwrap();
    for (index, parent, role, bits, rect) in [
        (
            0,
            NO_ACCESSIBLE_PARENT,
            AccessibleRole::Menu,
            0,
            Rect::new(0, 0, 200, 100),
        ),
        (
            1,
            0,
            AccessibleRole::MenuItem,
            AccessibleState::FOCUSED,
            Rect::new(8, 8, 184, 40),
        ),
        (
            2,
            0,
            AccessibleRole::MenuItem,
            AccessibleState::DISABLED,
            Rect::new(8, 52, 184, 40),
        ),
    ] {
        h.request(
            surface.client,
            Request::AccessibleNode {
                surface: surface.id,
                index,
                parent,
                role,
                state: state(bits),
                actions: Default::default(),
                rect,
            },
        )
        .unwrap();
    }
    let long = "a name longer than one thirty-two byte chunk, ñ included";
    for (index, text) in [(0, label), (1, long), (2, "Delete")] {
        for chunk in TextChunk::split(text) {
            h.request(
                surface.client,
                Request::AccessibleText {
                    surface: surface.id,
                    index,
                    field: AccessibleField::Name,
                    text: chunk,
                },
            )
            .unwrap();
        }
    }
    h.request(
        surface.client,
        Request::AccessibilityCommit {
            surface: surface.id,
        },
    )
    .unwrap();
}

/// What a reader received about `surface`, rebuilt: (role, name, global rect, state bits).
fn read_tree(h: &Harness, surface: SurfaceKey) -> Vec<(AccessibleRole, String, Rect, u16)> {
    let mut nodes: Vec<(AccessibleRole, String, Rect, u16)> = Vec::new();
    for event in h.events_for(READER) {
        match event {
            Event::AccessibilityTree {
                client,
                surface: id,
                ..
            } if client == surface.client && id == surface.id => nodes.clear(),
            Event::AccessibleNode {
                client,
                surface: id,
                role,
                state,
                rect,
                ..
            } if client == surface.client && id == surface.id => {
                nodes.push((role, String::new(), rect, state.bits()))
            }
            Event::AccessibleText {
                client,
                surface: id,
                index,
                field: AccessibleField::Name,
                text,
            } if client == surface.client && id == surface.id => {
                nodes[index as usize].1.push_str(text.as_str())
            }
            Event::AccessibilityRemoved {
                client,
                surface: id,
            } if client == surface.client && id == surface.id => nodes.clear(),
            _ => {}
        }
    }
    nodes
}

fn is_accessibility(event: &Event) -> bool {
    matches!(
        event,
        Event::AccessibilityTree { .. }
            | Event::AccessibleNode { .. }
            | Event::AccessibleText { .. }
            | Event::AccessibilityCommitted { .. }
            | Event::AccessibilityRemoved { .. }
    )
}

#[test]
fn a_reader_receives_committed_trees_and_ordinary_clients_never_do() {
    let mut h = Harness::new();
    h.scene.connect(READER, reader()).unwrap();
    let window = h.toplevel(APP, 1, 400, 300);
    let w = h.rect(window);
    h.clear_log();
    publish(&mut h, window, "Message actions");
    let tree = read_tree(&h, window);
    assert_eq!(tree.len(), 3);
    assert_eq!(tree[0].0, AccessibleRole::Menu);
    assert_eq!(tree[0].1, "Message actions");
    assert_eq!(
        tree[1].1,
        "a name longer than one thirty-two byte chunk, ñ included"
    );
    // Global rectangles for the reader.
    assert_eq!(tree[1].2, Rect::new(w.x + 8, w.y + 8, 184, 40));
    assert_eq!(tree[1].3, AccessibleState::FOCUSED);
    assert_eq!(tree[2].3, AccessibleState::DISABLED);
    for client in [APP, OTHER, SHELL, LOCKER] {
        assert!(
            !h.events_for(client).iter().any(is_accessibility),
            "client {client} saw an accessibility tree"
        );
    }
    // The compositor keeps surface-local rectangles.
    let snapshot = h.scene.accessibility_snapshot(window).unwrap();
    assert_eq!(snapshot[1].rect, Rect::new(8, 8, 184, 40));
    assert_eq!(snapshot[2].name, "Delete");
}

#[test]
fn malformed_snapshots_are_protocol_errors() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let node = |index, parent| Request::AccessibleNode {
        surface: 1,
        index,
        parent,
        role: AccessibleRole::Button,
        state: AccessibleState::default(),
        actions: Default::default(),
        rect: Rect::new(0, 0, 10, 10),
    };
    let begin = |nodes| Request::AccessibilityBegin { surface: 1, nodes };
    assert_eq!(
        h.request(APP, begin(MAX_ACCESSIBLE_NODES + 1)),
        Err(ErrorCode::QuotaExceeded)
    );
    assert_eq!(
        h.request(
            APP,
            Request::AccessibilityBegin {
                surface: 9,
                nodes: 1
            }
        ),
        Err(ErrorCode::UnknownSurface)
    );
    // Nodes before a snapshot began, and a commit without one.
    assert_eq!(
        h.request(APP, node(0, NO_ACCESSIBLE_PARENT)),
        Err(ErrorCode::InvalidOrder)
    );
    assert_eq!(
        h.request(APP, Request::AccessibilityCommit { surface: 1 }),
        Err(ErrorCode::InvalidOrder)
    );
    h.request(APP, begin(2)).unwrap();
    // A parent must come first: no self-parent, no forward parent, no cycle.
    assert_eq!(h.request(APP, node(1, 1)), Err(ErrorCode::InvalidMessage));
    assert_eq!(h.request(APP, node(0, 1)), Err(ErrorCode::InvalidMessage));
    assert_eq!(h.request(APP, node(2, 0)), Err(ErrorCode::InvalidMessage));
    h.request(APP, node(0, NO_ACCESSIBLE_PARENT)).unwrap();
    assert_eq!(
        h.request(APP, node(0, NO_ACCESSIBLE_PARENT)),
        Err(ErrorCode::InvalidMessage),
        "each node once"
    );
    // Text for a node not sent yet; a commit with a node missing.
    let text = |index| Request::AccessibleText {
        surface: 1,
        index,
        field: AccessibleField::Value,
        text: TextChunk::new("x").unwrap(),
    };
    assert_eq!(h.request(APP, text(1)), Err(ErrorCode::InvalidOrder));
    assert_eq!(
        h.request(APP, Request::AccessibilityCommit { surface: 1 }),
        Err(ErrorCode::InvalidOrder)
    );
    // Text is bounded per field.
    h.request(APP, begin(1)).unwrap();
    h.request(APP, node(0, NO_ACCESSIBLE_PARENT)).unwrap();
    let chunk = TextChunk::new(&"y".repeat(32)).unwrap();
    let field = Request::AccessibleText {
        surface: 1,
        index: 0,
        field: AccessibleField::Name,
        text: chunk,
    };
    for _ in 0..MAX_ACCESSIBLE_TEXT_BYTES / 32 {
        h.request(APP, field).unwrap();
    }
    assert_eq!(h.request(APP, field), Err(ErrorCode::QuotaExceeded));
    // Nothing was committed by any of this.
    assert!(h.scene.accessibility_snapshot(window).is_none());
}

#[test]
fn reader_actions_reach_the_owner_and_nobody_else_can_act() {
    let mut h = Harness::new();
    h.scene.connect(READER, reader()).unwrap();
    let window = h.toplevel(APP, 1, 400, 300);
    publish(&mut h, window, "Menu");
    h.clear_log();
    let act = |index, action| Request::AccessibilityAction {
        client: APP,
        surface: 1,
        index,
        action,
        custom: 0,
        start: 0,
        end: 0,
    };
    h.request(READER, act(1, AccessibleAction::Activate))
        .unwrap();
    let Some(Event::AccessibilityAction {
        surface,
        serial,
        index,
        action,
        ..
    }) = h.events_for(APP).last().copied()
    else {
        panic!("no action delivered");
    };
    assert_eq!((surface, index, action), (1, 1, AccessibleAction::Activate));
    // An activation is the user's input: the menu it opens may grab with its serial.
    let popup = h.popup(window, 2, menu(Rect::new(0, 0, 40, 20), 200, 150));
    h.request(APP, Request::GrabPopup { surface: 2, serial })
        .unwrap();
    h.present(popup);
    assert_eq!(h.scene.grab_chain(), &[popup]);
    h.scene.key(ESCAPE, true);
    h.pump();
    // A focus move is navigation: its serial authorizes no grab.
    h.clear_log();
    h.request(READER, act(2, AccessibleAction::Focus)).unwrap();
    let Some(Event::AccessibilityAction { serial, action, .. }) = h.events_for(APP).last().copied()
    else {
        panic!("no action delivered");
    };
    assert_eq!(action, AccessibleAction::Focus);
    let popup = h.popup(window, 3, menu(Rect::new(0, 0, 40, 20), 200, 150));
    h.request(APP, Request::GrabPopup { surface: 3, serial })
        .unwrap();
    h.pump();
    assert_eq!(h.dismissed(popup), Some(DismissReason::GrabDenied));
    // An ordinary client cannot act on another client's nodes.
    assert_eq!(
        h.request(OTHER, act(1, AccessibleAction::Activate)),
        Err(ErrorCode::Unauthorized)
    );
    // A node the snapshot does not have, or a surface that is gone, is answered with nothing.
    h.clear_log();
    h.request(READER, act(3, AccessibleAction::Activate))
        .unwrap();
    h.request(
        READER,
        Request::AccessibilityAction {
            client: APP,
            surface: 7,
            index: 0,
            action: AccessibleAction::Activate,
            custom: 0,
            start: 0,
            end: 0,
        },
    )
    .unwrap();
    // While locked, only lock surfaces can be acted on.
    h.scene.lock();
    h.request(READER, act(1, AccessibleAction::Activate))
        .unwrap();
    assert!(
        !h.events_for(APP)
            .iter()
            .any(|e| matches!(e, Event::AccessibilityAction { .. })),
        "{:?}",
        h.events_for(APP)
    );
}

#[test]
fn readers_follow_moves_removals_and_join_late() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let w = h.rect(window);
    publish(&mut h, window, "Menu");
    // A reader that connects later receives the current snapshot.
    h.scene.connect(READER, reader()).unwrap();
    h.pump();
    assert_eq!(read_tree(&h, window)[0].2, Rect::new(w.x, w.y, 200, 100));
    // The window moves: the reader receives the tree again with the new global rectangles.
    h.scene.move_window_to(window, (w.x + 50, w.y + 20));
    h.pump();
    assert_eq!(
        read_tree(&h, window)[1].2,
        Rect::new(w.x + 58, w.y + 28, 184, 40)
    );
    // The surface goes away: the reader drops it.
    h.request(APP, Request::Destroy { surface: 1 }).unwrap();
    assert!(read_tree(&h, window).is_empty());
    assert!(h.scene.accessibility_snapshot(window).is_none());
}

#[test]
fn damage_is_what_the_client_drew_since_the_frame_displayed() {
    let mut h = Harness::new();
    let window = h.toplevel(1, 1, 200, 100);
    let damage = |h: &mut Harness, rect: Rect| h.request(1, Request::Damage { surface: 1, rect });
    let presenting = |h: &mut Harness| {
        h.scene.begin_frame();
        let damage = h.scene.presenting_damage(window).cloned();
        h.scene.end_frame(1, Clock::Synthetic);
        h.pump();
        damage
    };
    // A Submit without Damage damages the whole surface.
    h.submit(window);
    assert_eq!(presenting(&mut h), Some(SurfaceDamage::Whole));
    // Damage, clipped to the buffer.
    damage(&mut h, Rect::new(10, 10, 20, 5)).unwrap();
    damage(&mut h, Rect::new(190, 90, 50, 50)).unwrap();
    h.submit(window);
    assert_eq!(
        presenting(&mut h),
        Some(SurfaceDamage::Rects(vec![
            Rect::new(10, 10, 20, 5),
            Rect::new(190, 90, 10, 10)
        ]))
    );
    // Two frames before one presents: the second replaces the first, and the damage of both is
    // what changed since the frame displayed.
    damage(&mut h, Rect::new(0, 0, 5, 5)).unwrap();
    h.submit(window);
    damage(&mut h, Rect::new(50, 50, 5, 5)).unwrap();
    h.submit(window);
    assert_eq!(
        presenting(&mut h),
        Some(SurfaceDamage::Rects(vec![
            Rect::new(0, 0, 5, 5),
            Rect::new(50, 50, 5, 5)
        ]))
    );
    // Past sixteen rectangles, the one holding them all.
    for i in 0..17 {
        damage(&mut h, Rect::new(i * 10, 0, 2, 2)).unwrap();
    }
    h.submit(window);
    assert_eq!(
        presenting(&mut h),
        Some(SurfaceDamage::Rects(vec![Rect::new(0, 0, 162, 2)]))
    );
    // An empty rectangle and an unknown surface are errors.
    assert_eq!(
        damage(&mut h, Rect::new(0, 0, 0, 5)),
        Err(ErrorCode::InvalidGeometry)
    );
    assert_eq!(
        h.request(
            1,
            Request::Damage {
                surface: 9,
                rect: Rect::new(0, 0, 1, 1)
            }
        ),
        Err(ErrorCode::UnknownSurface)
    );
}

#[test]
fn a_new_size_damages_the_whole_surface() {
    let mut h = Harness::new();
    let window = h.toplevel(1, 1, 200, 100);
    h.request(
        1,
        Request::Damage {
            surface: 1,
            rect: Rect::new(0, 0, 4, 4),
        },
    )
    .unwrap();
    h.submit(window);
    h.scene.begin_frame();
    assert_eq!(
        h.scene.presenting_damage(window),
        Some(&SurfaceDamage::Rects(vec![Rect::new(0, 0, 4, 4)]))
    );
    h.scene.end_frame(1, Clock::Synthetic);
    h.pump();
    // The compositor resizes the window; the client's next buffer is of the new size.
    h.scene.resize_window(window, Size::new(300, 150));
    h.pump();
    h.request(
        1,
        Request::Damage {
            surface: 1,
            rect: Rect::new(0, 0, 4, 4),
        },
    )
    .unwrap();
    h.submit(window);
    h.scene.begin_frame();
    assert_eq!(
        h.scene.presenting_damage(window),
        Some(&SurfaceDamage::Whole)
    );
}

// ---- clipboard (ADR 0007) -------------------------------------------------------------------

const PLAIN: &str = "text/plain;charset=utf-8";

fn mime(text: &str) -> TextChunk {
    TextChunk::new(text).unwrap()
}

/// A key typed into whichever client has focus: the serial its press was delivered with.
fn type_key(h: &mut Harness) -> u64 {
    h.scene.key(30, true);
    let pressed = h.pump();
    h.scene.key(30, false);
    h.pump();
    pressed
        .iter()
        .find_map(|(_, event)| match event {
            Event::Key {
                serial,
                pressed: true,
                ..
            } => Some(*serial),
            _ => None,
        })
        .expect("a key reached a client")
}

/// `client` copies: an offer of plain text with the serial of a key typed into it, committed.
fn copy(h: &mut Harness, client: ClientId, offer: u32) {
    let serial = type_key(h);
    for request in [
        Request::ClipboardOffer { offer, serial },
        Request::ClipboardType {
            offer,
            mime: mime(PLAIN),
        },
        Request::ClipboardCommit { offer },
    ] {
        h.request(client, request).unwrap();
    }
}

/// The clipboard events `client` received.
fn clipboard_events(h: &Harness, client: ClientId) -> Vec<Event> {
    h.events_for(client)
        .into_iter()
        .filter(|event| {
            matches!(
                event,
                Event::ClipboardTypes { .. }
                    | Event::ClipboardType { .. }
                    | Event::ClipboardEmpty {}
                    | Event::ClipboardSend { .. }
            )
        })
        .collect()
}

/// `client` pastes `selection` as `mime` with `serial`: the owner told to send, if allowed.
fn paste(
    h: &mut Harness,
    client: ClientId,
    selection: u32,
    mime_type: &str,
    serial: u64,
) -> Option<ClientId> {
    let request = Request::ClipboardReceive {
        selection,
        mime: mime(mime_type),
        serial,
    };
    h.scene.handle(client, request).unwrap();
    let owner = h.scene.take_transfer();
    h.pump();
    owner
}

/// Two windows, the first one focused, the log cleared.
fn two_windows(h: &mut Harness) -> (SurfaceKey, SurfaceKey) {
    let app = h.toplevel(APP, 1, 400, 300);
    let other = h.toplevel(OTHER, 1, 400, 300);
    // Apart, so that a click on one never lands on the other.
    h.scene.move_window_to(other, (1300, 100));
    h.present(other);
    let a = h.rect(app);
    h.click(a.x + 5, a.y + 5);
    assert_eq!(h.scene.focus(), Some(app));
    h.clear_log();
    (app, other)
}

#[test]
fn a_copy_reaches_the_focused_application_which_pastes_it_from_its_owner() {
    let mut h = Harness::new();
    let (_, other) = two_windows(&mut h);
    copy(&mut h, APP, 7);
    // The owner has focus: it is told what the clipboard now holds, under the compositor's name.
    assert_eq!(
        clipboard_events(&h, APP),
        [
            Event::ClipboardTypes {
                selection: 1,
                count: 1
            },
            Event::ClipboardType {
                selection: 1,
                mime: mime(PLAIN)
            },
        ]
    );
    assert!(clipboard_events(&h, OTHER).is_empty());
    // Focus moves: the application it reaches is told, once.
    let o = h.rect(other);
    h.click(o.x + 5, o.y + 5);
    assert_eq!(clipboard_events(&h, OTHER).len(), 2);
    h.clear_log();
    // A paste with a key of its own: the owner is asked to send that type of its offer.
    let serial = type_key(&mut h);
    assert_eq!(paste(&mut h, OTHER, 1, PLAIN, serial), Some(APP));
    assert_eq!(
        clipboard_events(&h, APP),
        [Event::ClipboardSend {
            offer: 7,
            mime: mime(PLAIN)
        }]
    );
    assert!(clipboard_events(&h, OTHER).is_empty());
}

#[test]
fn the_clipboard_is_read_and_replaced_only_by_the_user_s_own_actions() {
    let mut h = Harness::new();
    let (_, other) = two_windows(&mut h);
    // An offer with a serial of another client's input is refused: it takes no types, and
    // committing it changes nothing.
    let others_key = {
        let o = h.rect(other);
        h.click(o.x + 5, o.y + 5);
        type_key(&mut h)
    };
    h.clear_log();
    for request in [
        Request::ClipboardOffer {
            offer: 1,
            serial: others_key,
        },
        Request::ClipboardType {
            offer: 1,
            mime: mime(PLAIN),
        },
        Request::ClipboardCommit { offer: 1 },
    ] {
        h.request(APP, request).unwrap();
    }
    assert!(clipboard_events(&h, OTHER).is_empty());
    // An offer begun with a key of its own but committed after focus left is refused too.
    let app = key(APP, 1);
    let a = h.rect(app);
    h.click(a.x + 5, a.y + 5);
    let serial = type_key(&mut h);
    h.request(APP, Request::ClipboardOffer { offer: 2, serial })
        .unwrap();
    h.request(
        APP,
        Request::ClipboardType {
            offer: 2,
            mime: mime(PLAIN),
        },
    )
    .unwrap();
    let o = h.rect(other);
    h.click(o.x + 5, o.y + 5);
    h.clear_log();
    h.request(APP, Request::ClipboardCommit { offer: 2 })
        .unwrap();
    assert!(clipboard_events(&h, OTHER).is_empty());

    // A real copy, then pastes that must be refused.
    h.click(a.x + 5, a.y + 5);
    copy(&mut h, APP, 3);
    copy(&mut h, APP, 4);
    // The refused offers and commit created no content: these are the first and the second,
    // which replaced the first.
    let selection = h.scene.clipboard.as_ref().unwrap().selection;
    assert_eq!(selection, 2);
    let apps_old_key = type_key(&mut h);
    h.click(o.x + 5, o.y + 5);
    let others_key = type_key(&mut h);
    h.clear_log();
    // Without focus, even with a key of its own.
    assert_eq!(paste(&mut h, APP, selection, PLAIN, apps_old_key), None);
    // A type not offered.
    assert_eq!(
        paste(&mut h, OTHER, selection, "image/png", others_key),
        None
    );
    // Content the clipboard held, and no longer does.
    assert_eq!(paste(&mut h, OTHER, selection - 1, PLAIN, others_key), None);
    // A serial older than the client's latest 16 inputs.
    for _ in 0..RECENT_INPUTS {
        type_key(&mut h);
    }
    assert_eq!(paste(&mut h, OTHER, selection, PLAIN, others_key), None);
    assert!(clipboard_events(&h, APP).is_empty());
    // The same paste with a fresh key is allowed.
    let fresh = type_key(&mut h);
    assert_eq!(paste(&mut h, OTHER, selection, PLAIN, fresh), Some(APP));
}

#[test]
fn the_clipboard_empties_when_its_owner_clears_it_or_goes() {
    let mut h = Harness::new();
    let (app, other) = two_windows(&mut h);
    copy(&mut h, APP, 1);
    let serial = type_key(&mut h);
    h.clear_log();
    // Only the owner clears it, with an input of its own.
    h.request(OTHER, Request::ClipboardClear { serial })
        .unwrap();
    h.request(APP, Request::ClipboardClear { serial }).unwrap();
    assert_eq!(clipboard_events(&h, APP), [Event::ClipboardEmpty {}]);

    copy(&mut h, APP, 2);
    let o = h.rect(other);
    h.click(o.x + 5, o.y + 5);
    let others_key = type_key(&mut h);
    h.clear_log();
    // The owner disconnects: the focused application is told once, and nothing can be pasted.
    h.scene.disconnect(APP);
    h.pump();
    assert!(h.scene.focus() != Some(app));
    assert_eq!(clipboard_events(&h, OTHER), [Event::ClipboardEmpty {}]);
    assert_eq!(paste(&mut h, OTHER, 2, PLAIN, others_key), None);
}

#[test]
fn offers_and_their_types_are_bounded() {
    let mut h = Harness::new();
    two_windows(&mut h);
    let serial = type_key(&mut h);
    for offer in 0..MAX_CLIPBOARD_OFFERS as u32 {
        h.request(APP, Request::ClipboardOffer { offer, serial })
            .unwrap();
    }
    assert_eq!(
        h.request(APP, Request::ClipboardOffer { offer: 99, serial }),
        Err(ErrorCode::QuotaExceeded)
    );
    let mut h = Harness::new();
    two_windows(&mut h);
    let serial = type_key(&mut h);
    h.request(APP, Request::ClipboardOffer { offer: 1, serial })
        .unwrap();
    for index in 0..MAX_CLIPBOARD_TYPES {
        h.request(
            APP,
            Request::ClipboardType {
                offer: 1,
                mime: mime(&format!("text/x-{index}")),
            },
        )
        .unwrap();
    }
    assert_eq!(
        h.request(
            APP,
            Request::ClipboardType {
                offer: 1,
                mime: mime("text/x-more")
            }
        ),
        Err(ErrorCode::QuotaExceeded)
    );
}

// ---- the input method (ADR 0008) -------------------------------------------------------------

/// The input method, connected by the tests that need one.
const IME: ClientId = 6;
const KEY_A: u32 = 30;
const KEY_B: u32 = 48;

fn input_method() -> Grants {
    Grants {
        input_method: true,
        ..Grants::default()
    }
}

/// A focused window of APP whose field holds `text`, the caret at its end, and an input method.
fn served(text: &str) -> (Harness, SurfaceKey) {
    let mut h = Harness::new();
    h.scene.connect(IME, input_method()).unwrap();
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    enable_text_input(&mut h, window, text, 1);
    logged(&mut h);
    (h, window)
}

/// Every event since the last call, including those requests pumped on the way.
fn logged(h: &mut Harness) -> Vec<(ClientId, Event)> {
    h.pump();
    std::mem::take(&mut h.log)
}

fn to(events: &[(ClientId, Event)], client: ClientId) -> Vec<Event> {
    events
        .iter()
        .filter(|(c, _)| *c == client)
        .map(|(_, e)| *e)
        .collect()
}

/// The serial of the key the input method was last given.
fn held(events: &[(ClientId, Event)]) -> u64 {
    to(events, IME)
        .iter()
        .rev()
        .find_map(|e| match e {
            Event::InputMethodKey { serial, .. } => Some(*serial),
            _ => None,
        })
        .expect("a key for the input method")
}

#[test]
fn one_input_method_at_a_time() {
    let mut h = Harness::new();
    h.scene.connect(IME, input_method()).unwrap();
    assert_eq!(
        h.scene.connect(7, input_method()),
        Err(ErrorCode::Unauthorized)
    );
    h.scene.disconnect(IME);
    assert_eq!(h.scene.connect(7, input_method()), Ok(()));
    // Its requests are its own: another client sending one is refused.
    assert_eq!(
        h.request(APP, Request::InputMethodCommit { serial: 1 }),
        Err(ErrorCode::Unauthorized)
    );
}

#[test]
fn the_input_method_is_told_the_field_and_nothing_of_where_it_is() {
    let mut h = Harness::new();
    h.scene.connect(IME, input_method()).unwrap();
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    logged(&mut h);
    enable_text_input(&mut h, window, "hola", 1);
    let events = logged(&mut h);
    assert_eq!(
        to(&events, IME),
        [
            Event::InputMethodActivate {},
            Event::InputMethodPurpose {
                purpose: TextPurpose::Normal,
                hints: TextHints::default(),
            },
            Event::InputMethodSurrounding {
                text: TextChunk::new("hola").unwrap(),
            },
            Event::InputMethodSelection {
                anchor: 4,
                cursor: 4,
            },
            Event::InputMethodDone { serial: 1 },
        ]
    );
    // The field's next state, without activating again.
    h.request(
        APP,
        Request::TextInputSelection {
            surface: 1,
            anchor: 0,
            cursor: 4,
        },
    )
    .unwrap();
    h.request(
        APP,
        Request::TextInputCommit {
            surface: 1,
            serial: 2,
        },
    )
    .unwrap();
    let events = to(&logged(&mut h), IME);
    assert!(!events.contains(&Event::InputMethodActivate {}));
    assert!(events.contains(&Event::InputMethodSelection {
        anchor: 0,
        cursor: 4
    }));
    assert_eq!(events.last(), Some(&Event::InputMethodDone { serial: 2 }));
    // Enabling the field anew is another field: deactivated, then activated.
    enable_text_input(&mut h, window, "hola", 3);
    let events = to(&logged(&mut h), IME);
    assert_eq!(
        &events[..2],
        [
            Event::InputMethodDeactivate {},
            Event::InputMethodActivate {}
        ]
    );
}

#[test]
fn keys_go_to_the_input_method_first_and_those_it_gives_back_arrive_in_order() {
    let (mut h, _) = served("");
    h.scene.keyboard(KEY_A, true, Some("a"));
    let events = logged(&mut h);
    assert!(to(&events, APP).is_empty());
    let a = held(&events);
    assert_eq!(
        to(&events, IME),
        [Event::InputMethodKey {
            serial: a,
            key: KEY_A,
            pressed: true,
            text: PreeditText::new("a").unwrap(),
        }]
    );
    h.scene.keyboard(KEY_B, true, Some("b"));
    let b = held(&logged(&mut h));
    // The second answered first waits for the first.
    h.request(
        IME,
        Request::InputMethodKeyHandled {
            serial: b,
            used: false,
        },
    )
    .unwrap();
    assert!(to(&logged(&mut h), APP).is_empty());
    h.request(
        IME,
        Request::InputMethodKeyHandled {
            serial: a,
            used: false,
        },
    )
    .unwrap();
    let delivered: Vec<String> = to(&logged(&mut h), APP)
        .iter()
        .filter_map(|e| match e {
            Event::Key {
                key,
                serial,
                pressed: true,
                ..
            } => Some(format!("key {key} {}", *serial == a || *serial == b)),
            Event::TextInputCommitText { text, .. } => Some(text.as_str().to_string()),
            _ => None,
        })
        .collect();
    // Each key as if it had come straight: the key, then what it types; with its own serial.
    assert_eq!(
        delivered,
        [
            format!("key {KEY_A} true"),
            "a".into(),
            format!("key {KEY_B} true"),
            "b".into()
        ]
    );
    // A given-back key authorizes as a press does.
    assert!(h.scene.recent_input(APP, a));
}

#[test]
fn a_key_the_input_method_takes_becomes_its_change() {
    let (mut h, _) = served("");
    h.scene.keyboard(KEY_A, true, Some("a"));
    let a = held(&logged(&mut h));
    h.request(
        IME,
        Request::InputMethodKeyHandled {
            serial: a,
            used: true,
        },
    )
    .unwrap();
    for request in [
        Request::InputMethodPreeditText {
            text: TextChunk::new("あ").unwrap(),
        },
        Request::InputMethodPreeditCursor { begin: 3, end: 3 },
        Request::InputMethodPreeditSegment {
            begin: 0,
            end: 3,
            style: PreeditStyle::Composing,
        },
        Request::InputMethodCommit { serial: 1 },
    ] {
        h.request(IME, request).unwrap();
    }
    let events = to(&logged(&mut h), APP);
    assert!(!events.iter().any(|e| matches!(e, Event::Key { .. })));
    let (_, commit, preedit, cursor, segments, _, serial) =
        transaction(&events).expect("one transaction");
    assert_eq!(
        (commit.as_str(), preedit.as_str(), cursor, serial),
        ("", "あ", Some((3, 3)), 1)
    );
    assert_eq!(segments, [(0, 3, PreeditStyle::Composing)]);
}

#[test]
fn a_change_must_name_a_state_it_was_told_and_be_valid() {
    let (mut h, _) = served("");
    assert_eq!(
        h.request(IME, Request::InputMethodCommit { serial: 2 }),
        Err(ErrorCode::InvalidMessage)
    );
    let (mut h, _) = served("");
    h.request(
        IME,
        Request::InputMethodPreeditText {
            text: TextChunk::new("あ").unwrap(),
        },
    )
    .unwrap();
    h.request(IME, Request::InputMethodPreeditCursor { begin: 1, end: 1 })
        .unwrap();
    assert_eq!(
        h.request(IME, Request::InputMethodCommit { serial: 1 }),
        Err(ErrorCode::InvalidMessage)
    );
    // A change for an older state is passed on with the state it names: the field decides.
    let (mut h, window) = served("");
    enable_text_input(&mut h, window, "x", 2);
    logged(&mut h);
    h.request(
        IME,
        Request::InputMethodCommitText {
            text: TextChunk::new("y").unwrap(),
        },
    )
    .unwrap();
    h.request(IME, Request::InputMethodCommit { serial: 1 })
        .unwrap();
    let events = to(&logged(&mut h), APP);
    assert_eq!(transaction(&events).expect("one transaction").6, 1);
}

#[test]
fn a_password_never_reaches_the_input_method() {
    for (purpose, hints) in [
        (TextPurpose::Password, 0),
        (TextPurpose::Pin, 0),
        (TextPurpose::Normal, TextHints::SENSITIVE),
    ] {
        let (mut h, _) = served("abc");
        h.scene.keyboard(KEY_A, true, Some("a"));
        let a = held(&logged(&mut h));
        // The field becomes sensitive: the input method is deactivated and the key it held
        // goes on to the field.
        h.request(
            APP,
            Request::TextInputPurpose {
                surface: 1,
                purpose,
                hints: TextHints::new(hints).unwrap(),
            },
        )
        .unwrap();
        h.request(
            APP,
            Request::TextInputCommit {
                surface: 1,
                serial: 2,
            },
        )
        .unwrap();
        let events = logged(&mut h);
        assert_eq!(to(&events, IME), [Event::InputMethodDeactivate {}]);
        assert!(to(&events, APP)
            .iter()
            .any(|e| matches!(e, Event::Key { serial, .. } if *serial == a)));
        // Its keys go straight to the field, and nothing of it is told.
        h.scene.keyboard(KEY_B, true, Some("b"));
        let events = logged(&mut h);
        assert!(to(&events, IME).is_empty());
        assert!(to(&events, APP)
            .iter()
            .any(|e| matches!(e, Event::Key { key: KEY_B, .. })));
        // The answer to a key it no longer holds changes nothing.
        h.request(
            IME,
            Request::InputMethodKeyHandled {
                serial: a,
                used: false,
            },
        )
        .unwrap();
        assert!(logged(&mut h).is_empty());
    }
}

#[test]
fn focus_leaving_deactivates_it_before_the_next_field_and_its_keys_go_first() {
    let (mut h, window) = served("uno");
    let other = h.toplevel(OTHER, 1, 400, 300);
    enable_text_input(&mut h, other, "dos", 1);
    // Mapped after nothing was typed, it took focus: back to the first field.
    h.activate(window);
    logged(&mut h);
    h.scene.keyboard(KEY_A, true, Some("a"));
    let a = held(&logged(&mut h));
    h.scene.activate(other);
    let events = logged(&mut h);
    // The held key reaches the field it was typed into, before that field loses focus.
    let app = to(&events, APP);
    let key = app
        .iter()
        .position(|e| matches!(e, Event::Key { serial, .. } if *serial == a));
    let out = app
        .iter()
        .position(|e| matches!(e, Event::Focus { focused: false, .. }));
    assert!(key.is_some() && key < out, "{app:?}");
    // The input method is told the old field is gone before anything of the new one.
    let ime = to(&events, IME);
    assert_eq!(
        &ime[..2],
        [
            Event::InputMethodDeactivate {},
            Event::InputMethodActivate {}
        ]
    );
    assert!(ime.contains(&Event::InputMethodSurrounding {
        text: TextChunk::new("dos").unwrap()
    }));
}

#[test]
fn shortcuts_and_an_unserved_keyboard_bypass_the_input_method() {
    let (mut h, window) = served("");
    h.request(
        SHELL,
        Request::CreateShellSurface {
            surface: 1,
            kind: ShellKind::Panel,
            edge: PanelEdge::Bottom,
            output: 1,
            size: Size::new(1, 48),
            exclusive: 48,
        },
    )
    .unwrap();
    h.present(key(SHELL, 1));
    h.activate(window);
    logged(&mut h);
    h.scene.keyboard(SUPER, true, None);
    let events = logged(&mut h);
    assert!(to(&events, IME).is_empty());
    assert!(to(&events, SHELL)
        .iter()
        .any(|e| matches!(e, Event::Key { key: SUPER, .. })));
    // A field without text input is not served: keys go straight, text as Text.
    let (mut h, _) = served("");
    h.request(APP, Request::TextInputDisable { surface: 1 })
        .unwrap();
    h.request(
        APP,
        Request::TextInputCommit {
            surface: 1,
            serial: 2,
        },
    )
    .unwrap();
    logged(&mut h);
    h.scene.keyboard(KEY_A, true, Some("a"));
    let events = logged(&mut h);
    assert!(to(&events, IME).is_empty());
    assert!(to(&events, APP)
        .iter()
        .any(|e| matches!(e, Event::Text { .. })));
}

#[test]
fn a_stuck_input_method_does_not_take_the_keyboard_with_it() {
    let (mut h, _) = served("");
    for _ in 0..MAX_PENDING_KEYS {
        h.scene.keyboard(KEY_A, true, Some("a"));
    }
    assert!(to(&logged(&mut h), APP).is_empty());
    // One more than it may hold: everything it holds goes on.
    h.scene.keyboard(KEY_A, true, Some("a"));
    let keys = to(&logged(&mut h), APP)
        .iter()
        .filter(|e| matches!(e, Event::Key { .. }))
        .count();
    assert_eq!(keys, MAX_PENDING_KEYS + 1);
    // And it goes, holding a key: that key goes on too.
    h.scene.keyboard(KEY_B, true, Some("b"));
    logged(&mut h);
    h.scene.disconnect(IME);
    assert!(to(&logged(&mut h), APP)
        .iter()
        .any(|e| matches!(e, Event::Key { key: KEY_B, .. })));
    // With no input method, the keyboard types straight.
    h.scene.keyboard(KEY_A, true, Some("a"));
    assert!(to(&logged(&mut h), APP)
        .iter()
        .any(|e| matches!(e, Event::Key { key: KEY_A, .. })));
}

/// The field of `window` says its caret is at `caret` (surface-local), as commit `serial`.
fn move_caret(h: &mut Harness, window: SurfaceKey, caret: Rect, serial: u32) {
    h.request(
        window.client,
        Request::TextInputCaret {
            surface: window.id,
            rect: caret,
        },
    )
    .unwrap();
    h.request(
        window.client,
        Request::TextInputCommit {
            surface: window.id,
            serial,
        },
    )
    .unwrap();
}

/// A served field with its caret at (10, 20), and the input method's shown candidates.
fn candidates() -> (Harness, SurfaceKey, SurfaceKey) {
    let (mut h, window) = served("");
    move_caret(&mut h, window, Rect::new(10, 20, 2, 18), 2);
    h.request(
        IME,
        Request::CreateInputCandidates {
            surface: 9,
            size: Size::new(240, 120),
        },
    )
    .unwrap();
    let candidates = key(IME, 9);
    h.present(candidates);
    logged(&mut h);
    (h, window, candidates)
}

#[test]
fn only_the_input_method_makes_candidates_and_only_one() {
    let (mut h, _, _) = candidates();
    assert_eq!(
        h.request(
            APP,
            Request::CreateInputCandidates {
                surface: 5,
                size: Size::new(10, 10),
            }
        ),
        Err(ErrorCode::Unauthorized)
    );
    assert_eq!(
        h.request(
            IME,
            Request::CreateInputCandidates {
                surface: 10,
                size: Size::new(10, 10),
            }
        ),
        Err(ErrorCode::InvalidRole)
    );
}

#[test]
fn candidates_sit_below_the_caret_and_their_configure_says_only_their_size() {
    let (mut h, window) = served("");
    move_caret(&mut h, window, Rect::new(10, 20, 2, 18), 2);
    logged(&mut h);
    h.request(
        IME,
        Request::CreateInputCandidates {
            surface: 9,
            size: Size::new(240, 120),
        },
    )
    .unwrap();
    let configures: Vec<Rect> = to(&logged(&mut h), IME)
        .iter()
        .filter_map(|e| match e {
            Event::Configure { rect, .. } => Some(*rect),
            _ => None,
        })
        .collect();
    assert_eq!(configures, [Rect::new(0, 0, 240, 120)]);
    h.present(key(IME, 9));
    let w = h.rect(window);
    // Below the caret's bottom-left corner.
    assert_eq!(h.rect(key(IME, 9)), Rect::new(w.x + 10, w.y + 38, 240, 120));
}

#[test]
fn candidates_follow_the_caret_and_the_window() {
    let (mut h, window, candidates) = candidates();
    let w = h.rect(window);
    move_caret(&mut h, window, Rect::new(60, 20, 2, 18), 3);
    assert_eq!(h.rect(candidates), Rect::new(w.x + 60, w.y + 38, 240, 120));
    h.scene.move_window_to(window, (w.x + 100, w.y + 50));
    let moved = h.rect(window);
    assert_eq!((moved.x, moved.y), (w.x + 100, w.y + 50));
    assert_eq!(
        h.rect(candidates),
        Rect::new(moved.x + 60, moved.y + 38, 240, 120)
    );
    // Near the output's bottom there is no room below: above the caret instead.
    h.scene
        .move_window_to(window, (moved.x, SCREEN.bottom() as i32 - 100));
    let low = h.rect(window);
    assert_eq!(
        h.rect(candidates),
        Rect::new(low.x + 60, low.y + 20 - 120, 240, 120)
    );
}

#[test]
fn candidates_show_only_while_a_field_is_served() {
    let (mut h, window, candidates) = candidates();
    assert!(h.rendered(candidates));
    // A password is not served: the candidates go.
    h.request(
        APP,
        Request::TextInputPurpose {
            surface: 1,
            purpose: TextPurpose::Password,
            hints: TextHints::default(),
        },
    )
    .unwrap();
    h.request(
        APP,
        Request::TextInputCommit {
            surface: 1,
            serial: 3,
        },
    )
    .unwrap();
    assert!(!h.rendered(candidates));
    // An ordinary field again, at a known caret: back.
    enable_text_input(&mut h, window, "", 4);
    move_caret(&mut h, window, Rect::new(10, 20, 2, 18), 5);
    assert!(h.rendered(candidates));
    // With the input method's grant revoked they go, and stay gone.
    h.scene.revoke(
        IME,
        Grants {
            input_method: true,
            ..Grants::default()
        },
    );
    h.pump();
    assert!(!h.rendered(candidates));
}

#[test]
fn a_press_on_the_candidates_reaches_the_input_method_and_leaves_focus_in_the_field() {
    let (mut h, window, candidates) = candidates();
    let c = h.rect(candidates);
    h.click(c.x + 20, c.y + 20);
    let events = logged(&mut h);
    assert!(to(&events, IME).iter().any(|e| matches!(
        e,
        Event::Pointer {
            surface: 9,
            kind: PointerKind::Press,
            ..
        }
    )));
    assert!(!to(&events, IME).contains(&Event::InputMethodDeactivate {}));
    assert_eq!(h.scene.focus(), Some(window));
    // It still serves the field: a key goes to it first.
    h.scene.keyboard(KEY_A, true, Some("a"));
    assert!(to(&logged(&mut h), IME)
        .iter()
        .any(|e| matches!(e, Event::InputMethodKey { .. })));
}

// ---- workspaces (ADR 0009) -------------------------------------------------------------------

#[test]
fn only_the_active_workspaces_windows_are_shown_and_focused() {
    let mut h = Harness::new();
    let first = h.toplevel(APP, 1, 400, 300);
    assert_eq!(h.scene.add_workspace(), Some(1));
    assert!(h.scene.activate_workspace(1));
    h.pump();
    // The window of workspace 0 is gone from the screen, and focus with it.
    assert!(!h.rendered(first));
    assert_eq!(h.scene.focus(), None);
    // A window opens on the active workspace, and takes focus there.
    let second = h.toplevel(OTHER, 1, 400, 300);
    assert_eq!(h.scene.window_workspace(second), Some(1));
    assert_eq!(h.scene.focus(), Some(second));
    // Back: the first is shown and focused, the second hidden.
    assert!(h.scene.activate_workspace(0));
    h.pump();
    assert!(h.rendered(first) && !h.rendered(second));
    assert_eq!(h.scene.focus(), Some(first));
    assert_eq!(h.scene.workspaces(), (2, 0));
    // A workspace that does not exist is refused.
    assert!(!h.scene.activate_workspace(2));
}

#[test]
fn a_window_on_another_workspace_takes_no_input() {
    let mut h = Harness::new();
    let below = h.toplevel(OTHER, 1, 800, 600);
    let above = h.toplevel(APP, 1, 400, 300);
    let a = h.rect(above);
    h.scene.add_workspace();
    assert!(h.scene.move_to_workspace(above, 1));
    h.pump();
    // A press where it was reaches the window under it.
    logged(&mut h);
    h.click(a.x + 10, a.y + 10);
    let events = logged(&mut h);
    assert!(events.iter().any(|(client, e)| *client == OTHER
        && matches!(
            e,
            Event::Pointer {
                kind: PointerKind::Press,
                ..
            }
        )));
    assert!(!events.iter().any(|(client, _)| *client == APP));
    assert_eq!(h.scene.focus(), Some(below));
    // Keys go to the focused window shown, never to a hidden one.
    h.scene.key(30, true);
    let keys: Vec<ClientId> = logged(&mut h)
        .iter()
        .filter(|(_, e)| matches!(e, Event::Key { .. }))
        .map(|(client, _)| *client)
        .collect();
    assert_eq!(keys, [OTHER]);
}

#[test]
fn leaving_a_workspace_closes_its_menus_and_moving_a_window_moves_focus() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let w = h.rect(window);
    let popup = h.open_menu(
        window,
        2,
        (w.x + 10, w.y + 10),
        menu(Rect::new(0, 0, 4, 4), 200, 100),
    );
    assert_eq!(h.scene.grab_chain(), &[popup]);
    h.scene.add_workspace();
    assert!(h.scene.activate_workspace(1));
    h.pump();
    assert_eq!(h.dismissed(popup), Some(DismissReason::ParentHidden));
    assert!(h.scene.grab_chain().is_empty());
    // Moving the focused window away: it goes, focus goes to the topmost window left.
    assert!(h.scene.activate_workspace(0));
    let other = h.toplevel(OTHER, 1, 300, 200);
    h.activate(window);
    assert!(h.scene.move_to_workspace(window, 1));
    h.pump();
    assert!(!h.rendered(window));
    assert_eq!(h.scene.focus(), Some(other));
    // Moved back, it is shown without taking focus.
    assert!(h.scene.move_to_workspace(window, 0));
    h.pump();
    assert!(h.rendered(window));
    assert_eq!(h.scene.focus(), Some(other));
    // Only windows move: a popup, a missing workspace or a missing window is refused.
    assert!(!h.scene.move_to_workspace(popup, 1));
    assert!(!h.scene.move_to_workspace(window, 5));
    assert!(!h.scene.move_to_workspace(key(APP, 9), 1));
}

#[test]
fn removing_a_workspace_moves_its_windows_to_its_neighbour() {
    let mut h = Harness::new();
    let on0 = h.toplevel(APP, 1, 300, 200);
    for _ in 0..3 {
        h.scene.add_workspace();
    }
    h.scene.activate_workspace(2);
    let on2 = h.toplevel(APP, 2, 300, 200);
    h.scene.activate_workspace(3);
    let on3 = h.toplevel(APP, 3, 300, 200);
    // Removing the active workspace 3: its window goes to 2, which is now active.
    assert!(h.scene.remove_workspace(3));
    assert_eq!(h.scene.workspaces(), (3, 2));
    assert_eq!(h.scene.window_workspace(on3), Some(2));
    assert!(h.rendered(on2) && h.rendered(on3) && !h.rendered(on0));
    // Removing workspace 1, before the active one: the numbers after it go down.
    assert!(h.scene.remove_workspace(1));
    assert_eq!(h.scene.workspaces(), (2, 1));
    assert_eq!(h.scene.window_workspace(on2), Some(1));
    // Removing the first: its windows go to the one after it, which becomes the first.
    assert!(h.scene.remove_workspace(0));
    assert_eq!(h.scene.workspaces(), (1, 0));
    assert_eq!(h.scene.window_workspace(on0), Some(0));
    assert!(h.rendered(on0) && h.rendered(on2));
    // The last one stays.
    assert!(!h.scene.remove_workspace(0));
    // And there are at most MAX_WORKSPACES.
    for _ in 1..MAX_WORKSPACES {
        assert!(h.scene.add_workspace().is_some());
    }
    assert_eq!(h.scene.add_workspace(), None);
}

#[test]
fn workspaces_stay_while_the_session_is_locked() {
    let mut h = Harness::new();
    h.toplevel(APP, 1, 300, 200);
    h.scene.add_workspace();
    h.scene.lock();
    assert!(!h.scene.activate_workspace(1));
    assert!(!h.scene.remove_workspace(1));
    assert_eq!(h.scene.workspaces(), (2, 0));
}

// ---- the shell's windows and workspaces (ADR 0009) -------------------------------------------

/// Names a window: its title and application id, each in one chunk.
fn name(h: &mut Harness, window: SurfaceKey, title: &str, app_id: &str) {
    for (app, text) in [(false, title), (true, app_id)] {
        let text = PreeditText::new(text).unwrap();
        let request = if app {
            Request::SetAppId {
                surface: window.id,
                index: 0,
                last: true,
                text,
            }
        } else {
            Request::SetTitle {
                surface: window.id,
                index: 0,
                last: true,
                text,
            }
        };
        h.request(window.client, request).unwrap();
    }
}

/// What a client was told, but the menu bar (ADR 0021).
fn windows_events(events: &[(ClientId, Event)], client: ClientId) -> Vec<Event> {
    to(events, client)
        .into_iter()
        .filter(|event| !is_menu_bar(event))
        .collect()
}

fn is_menu_bar(event: &Event) -> bool {
    matches!(
        event,
        Event::MenuBar { .. }
            | Event::MenuBarItem { .. }
            | Event::MenuBarText { .. }
            | Event::MenuBarCommitted { .. }
    )
}

/// The windows a shell client was told of, by number: (workspace, minimized, focused, title, app
/// id), from every event it received; and the workspaces last told.
type ToldWindows = std::collections::BTreeMap<u32, (u32, bool, bool, String, String)>;
fn told(events: &[(ClientId, Event)], client: ClientId) -> (ToldWindows, Option<(u32, u32)>) {
    let mut windows = ToldWindows::new();
    let mut workspaces = None;
    let mut title = String::new();
    let mut app = String::new();
    for event in to(events, client) {
        match event {
            Event::WindowState {
                window,
                workspace,
                minimized,
                focused,
                launched: _,
            } => {
                let entry = windows.entry(window).or_default();
                (entry.0, entry.1, entry.2) = (workspace, minimized, focused);
            }
            Event::WindowTitle {
                window,
                index,
                last,
                text,
            } => {
                if index == 0 {
                    title.clear();
                }
                title.push_str(text.as_str());
                if last {
                    windows.entry(window).or_default().3 = title.clone();
                }
            }
            Event::WindowAppId {
                window,
                index,
                last,
                text,
            } => {
                if index == 0 {
                    app.clear();
                }
                app.push_str(text.as_str());
                if last {
                    windows.entry(window).or_default().4 = app.clone();
                }
            }
            Event::WindowClosed { window } => {
                windows.remove(&window);
            }
            Event::Workspaces { count, active } => workspaces = Some((count, active)),
            _ => {}
        }
    }
    (windows, workspaces)
}

#[test]
fn the_shell_is_told_each_window_and_what_changes_and_applications_are_not() {
    let mut h = Harness::new();
    // The shell is told the workspaces at once, before any window.
    let first = logged(&mut h);
    assert_eq!(told(&first, SHELL), (ToldWindows::new(), Some((1, 0))));
    let inbox = h.toplevel(APP, 1, 400, 300);
    name(&mut h, inbox, "Bandeja de entrada", "so.oxy.inbox");
    let other = h.toplevel(OTHER, 1, 400, 300);
    let events = logged(&mut h);
    let (windows, workspaces) = told(&events, SHELL);
    let (a, b) = (
        h.scene.window_number(inbox).unwrap(),
        h.scene.window_number(other).unwrap(),
    );
    assert_ne!(a, b);
    assert_eq!(
        windows[&a],
        (
            0,
            false,
            false,
            "Bandeja de entrada".into(),
            "so.oxy.inbox".into()
        )
    );
    assert_eq!(windows[&b], (0, false, true, String::new(), String::new()));
    assert_eq!(workspaces, None);
    // Applications are never told of windows.
    for client in [APP, OTHER] {
        assert!(to(&events, client).iter().all(|e| !matches!(
            e,
            Event::WindowState { .. }
                | Event::WindowTitle { .. }
                | Event::WindowAppId { .. }
                | Event::WindowClosed { .. }
                | Event::Workspaces { .. }
        )));
    }
    // Focus moving and a window going: what changed, and nothing else (of windows; the menu
    // bar follows the active window, `menu_bar_*` below).
    h.activate(inbox);
    let events = logged(&mut h);
    assert_eq!(
        windows_events(&events, SHELL),
        [
            Event::WindowState {
                window: a,
                workspace: 0,
                minimized: false,
                focused: true,
                launched: false,
            },
            Event::WindowState {
                window: b,
                workspace: 0,
                minimized: false,
                focused: false,
                launched: false,
            },
        ]
    );
    h.request(OTHER, Request::Destroy { surface: 1 }).unwrap();
    let events = logged(&mut h);
    assert_eq!(to(&events, SHELL), [Event::WindowClosed { window: b }]);
    // A shell that connects later is told everything there is.
    h.scene
        .connect(
            7,
            Grants {
                shell: true,
                ..Grants::default()
            },
        )
        .unwrap();
    let (windows, workspaces) = told(&logged(&mut h), 7);
    assert_eq!(windows.keys().copied().collect::<Vec<_>>(), [a]);
    assert_eq!(windows[&a].3, "Bandeja de entrada");
    assert_eq!(workspaces, Some((1, 0)));
}

#[test]
fn titles_come_in_order_within_their_limit_from_a_windows_owner() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    logged(&mut h);
    let chunk = |text: &str| PreeditText::new(text).unwrap();
    let title = |index: u8, last: bool, text: &str| Request::SetTitle {
        surface: 1,
        index,
        last,
        text: chunk(text),
    };
    // A title of two chunks.
    h.request(APP, title(0, false, &"a".repeat(32))).unwrap();
    h.request(APP, title(1, true, "b")).unwrap();
    let (windows, _) = told(&logged(&mut h), SHELL);
    let number = h.scene.window_number(window).unwrap();
    assert_eq!(windows[&number].3, format!("{}b", "a".repeat(32)));
    // A chunk out of order, a title too long, or a surface that is not a window.
    h.request(APP, title(0, false, "x")).unwrap();
    assert_eq!(
        h.request(APP, title(2, true, "y")),
        Err(ErrorCode::InvalidOrder)
    );
    let mut h = Harness::new();
    h.toplevel(APP, 1, 400, 300);
    for index in 0..8 {
        h.request(APP, title(index, false, &"a".repeat(32)))
            .unwrap();
    }
    assert_eq!(
        h.request(APP, title(8, true, "a")),
        Err(ErrorCode::QuotaExceeded)
    );
    let mut h = Harness::new();
    let w = h.toplevel(APP, 1, 400, 300);
    let r = h.rect(w);
    h.open_menu(
        w,
        2,
        (r.x + 10, r.y + 10),
        menu(Rect::new(0, 0, 4, 4), 200, 100),
    );
    let on_popup = Request::SetTitle {
        surface: 2,
        index: 0,
        last: true,
        text: chunk("menu"),
    };
    assert_eq!(h.request(APP, on_popup), Err(ErrorCode::InvalidRole));
    let missing = Request::SetTitle {
        surface: 9,
        index: 0,
        last: true,
        text: chunk("menu"),
    };
    assert_eq!(h.request(APP, missing), Err(ErrorCode::UnknownSurface));
}

#[test]
fn the_shell_focuses_minimizes_moves_and_closes_windows_and_manages_workspaces() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let other = h.toplevel(OTHER, 1, 400, 300);
    let number = h.scene.window_number(window).unwrap();
    let shell = |h: &mut Harness, request| h.request(SHELL, request).unwrap();
    // Workspaces: add one, move the window there, switch to it.
    shell(&mut h, Request::AddWorkspace {});
    assert_eq!(h.scene.workspaces(), (2, 0));
    shell(
        &mut h,
        Request::MoveWindowToWorkspace {
            window: number,
            workspace: 1,
        },
    );
    assert!(!h.rendered(window));
    // Minimized there, then focused from workspace 0: its workspace is shown, it is restored
    // and focused.
    shell(&mut h, Request::MinimizeWindow { window: number });
    shell(&mut h, Request::FocusWindow { window: number });
    assert_eq!(h.scene.workspaces(), (2, 1));
    assert!(h.rendered(window) && !h.rendered(other));
    assert_eq!(h.scene.focus(), Some(window));
    let (windows, workspaces) = told(&logged(&mut h), SHELL);
    assert_eq!(windows[&number].0, 1);
    assert!(!windows[&number].1 && windows[&number].2);
    assert_eq!(workspaces, Some((2, 1)));
    // Close: the application is asked, and decides.
    shell(&mut h, Request::CloseWindow { window: number });
    assert_eq!(
        to(&logged(&mut h), APP),
        [Event::CloseRequested { surface: 1 }]
    );
    assert!(h.rendered(window));
    // Removing the workspace takes the window to its neighbour.
    shell(&mut h, Request::RemoveWorkspace { workspace: 1 });
    assert_eq!(h.scene.workspaces(), (1, 0));
    assert!(h.rendered(window) && h.rendered(other));
    // A window that has gone is ignored; only the shell may ask.
    shell(&mut h, Request::FocusWindow { window: 999 });
    assert_eq!(
        h.request(APP, Request::FocusWindow { window: number }),
        Err(ErrorCode::Unauthorized)
    );
    assert_eq!(
        h.request(APP, Request::AddWorkspace {}),
        Err(ErrorCode::Unauthorized)
    );
}

#[test]
fn the_pointer_takes_the_shape_the_surface_under_it_wants() {
    use oxyos_protocol::PointerShape;
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let w = h.rect(window);
    let other = h.toplevel(OTHER, 1, 100, 100);
    let o = h.rect(other);
    assert_eq!(h.scene.pointer_shape(), PointerShape::Arrow);
    h.request(
        APP,
        Request::SetPointerShape {
            surface: 1,
            shape: PointerShape::Text,
        },
    )
    .unwrap();
    // Over the window, its shape; over the other one, which asked for none, the arrow.
    h.scene
        .pointer_motion(w.x + w.width as i32 - 5, w.y + w.height as i32 - 5);
    assert_eq!(h.scene.pointer_shape(), PointerShape::Text);
    h.scene.pointer_motion(o.x + 5, o.y + 5);
    assert_eq!(h.scene.pointer_shape(), PointerShape::Arrow);
    // Surface 1 of another client is its own: asking for a shape changes only it.
    h.request(
        OTHER,
        Request::SetPointerShape {
            surface: 1,
            shape: PointerShape::Hand,
        },
    )
    .unwrap();
    assert_eq!(h.scene.pointer_shape(), PointerShape::Hand);
    h.scene
        .pointer_motion(w.x + w.width as i32 - 5, w.y + w.height as i32 - 5);
    assert_eq!(h.scene.pointer_shape(), PointerShape::Text);
    assert_eq!(
        h.request(
            APP,
            Request::SetPointerShape {
                surface: 9,
                shape: PointerShape::Hand,
            },
        ),
        Err(ErrorCode::UnknownSurface)
    );
    // While the compositor moves the window, the move shape.
    let serial = h.press(w.x + w.width as i32 - 5, w.y + w.height as i32 - 5);
    h.request(APP, Request::BeginMove { surface: 1, serial })
        .unwrap();
    h.scene
        .pointer_motion(w.x + w.width as i32 + 20, w.y + w.height as i32);
    assert_eq!(h.scene.pointer_shape(), PointerShape::Move);
}

impl Harness {
    /// The locker's lock surface on the output, shown.
    fn lock_surface(&mut self, id: u32) -> SurfaceKey {
        self.request(
            LOCKER,
            Request::CreateLockSurface {
                surface: id,
                output: 1,
            },
        )
        .unwrap();
        self.present(key(LOCKER, id));
        key(LOCKER, id)
    }

    fn give_password(&mut self, surface: SurfaceKey, password: &str) -> Result<(), ErrorCode> {
        for secret in oxyos_protocol::SecretChunk::split(password) {
            self.request(
                surface.client,
                Request::AuthenticateSecret {
                    surface: surface.id,
                    secret,
                },
            )?;
        }
        self.request(
            surface.client,
            Request::Authenticate {
                surface: surface.id,
            },
        )
    }

    fn results(&self, client: ClientId) -> Vec<(u32, oxyos_protocol::AuthOutcome, u32)> {
        self.events_for(client)
            .into_iter()
            .filter_map(|event| match event {
                Event::AuthenticationResult {
                    surface,
                    outcome,
                    wait_s,
                } => Some((surface, outcome, wait_s)),
                _ => None,
            })
            .collect()
    }
}

#[test]
fn the_shell_locks_and_the_lock_screen_is_told() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    h.clear_log();
    // Only the shell may lock.
    assert_eq!(
        h.request(APP, Request::LockSession {}),
        Err(ErrorCode::Unauthorized)
    );
    assert!(!h.scene.is_locked());
    h.request(SHELL, Request::LockSession {}).unwrap();
    assert!(h.scene.is_locked());
    assert!(!h.rendered(window));
    assert_eq!(
        h.events_for(LOCKER),
        vec![Event::LockState { locked: true }]
    );
    // Telling is the lock grant's: the shell and applications are not told.
    assert!(!h
        .events_for(APP)
        .iter()
        .chain(h.events_for(SHELL).iter())
        .any(|event| matches!(event, Event::LockState { .. })));
    // Locking again changes nothing and tells nothing.
    h.clear_log();
    h.request(SHELL, Request::LockSession {}).unwrap();
    assert!(h.events_for(LOCKER).is_empty());
    h.scene.unlock();
    h.pump();
    assert_eq!(
        h.events_for(LOCKER),
        vec![Event::LockState { locked: false }]
    );
    assert!(h.rendered(window));
}

#[test]
fn a_lock_screen_that_connects_while_locked_is_told_at_once() {
    let mut h = Harness::new();
    h.scene.lock();
    h.scene.disconnect(LOCKER);
    h.clear_log();
    h.scene
        .connect(
            LOCKER,
            Grants {
                lock: true,
                ..Grants::default()
            },
        )
        .unwrap();
    h.pump();
    assert_eq!(
        h.events_for(LOCKER),
        vec![Event::LockState { locked: true }]
    );
    // An application connecting is told nothing of it.
    h.scene.connect(9, Grants::default()).unwrap();
    h.pump();
    assert!(h.events_for(9).is_empty());
}

#[test]
fn a_lock_surfaces_password_goes_to_the_session_and_its_answer_back() {
    let mut h = Harness::new();
    h.scene.lock();
    let surface = h.lock_surface(1);
    let password = "correct horse battery staple, ñandú";
    h.give_password(surface, password).unwrap();
    let (from, secret) = h.scene.take_authentication().unwrap();
    assert_eq!(from, surface);
    assert_eq!(&secret[..], password.as_bytes());
    assert!(h.scene.take_authentication().is_none());
    // Another password while that one is checked is busy, and goes nowhere.
    h.give_password(surface, "another").unwrap();
    assert!(h.scene.take_authentication().is_none());
    assert_eq!(
        h.results(LOCKER),
        vec![(1, oxyos_protocol::AuthOutcome::Busy, 0)]
    );
    // The answer goes to the surface; it does not unlock.
    h.clear_log();
    h.scene
        .authenticated(surface, oxyos_protocol::AuthOutcome::Throttled, 30);
    h.pump();
    assert_eq!(
        h.results(LOCKER),
        vec![(1, oxyos_protocol::AuthOutcome::Throttled, 30)]
    );
    assert!(h.scene.is_locked());
    // The next one is checked.
    h.give_password(surface, password).unwrap();
    assert!(h.scene.take_authentication().is_some());
}

#[test]
fn only_a_lock_surface_while_locked_gives_a_password() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    h.scene.lock();
    assert_eq!(
        h.give_password(window, "guess"),
        Err(ErrorCode::InvalidRole)
    );
    assert_eq!(
        h.request(APP, Request::Authenticate { surface: 7 }),
        Err(ErrorCode::UnknownSurface)
    );
    assert!(h.scene.take_authentication().is_none());
    // Unlocked, a lock surface's password is dropped: there is nothing to unlock.
    let surface = h.lock_surface(1);
    h.scene.unlock();
    h.give_password(surface, "late").unwrap();
    assert!(h.scene.take_authentication().is_none());
    // Typed while locked and submitted after the unlock, it is dropped as well.
    h.scene.lock();
    h.request(
        LOCKER,
        Request::AuthenticateSecret {
            surface: 1,
            secret: oxyos_protocol::SecretChunk::new("half").unwrap(),
        },
    )
    .unwrap();
    h.scene.unlock();
    h.scene.lock();
    h.request(LOCKER, Request::Authenticate { surface: 1 })
        .unwrap();
    let (_, secret) = h.scene.take_authentication().unwrap();
    assert!(
        secret.is_empty(),
        "the half given before the unlock is gone"
    );
}

#[test]
fn a_password_longer_than_pam_takes_is_a_protocol_error() {
    let mut h = Harness::new();
    h.scene.lock();
    let surface = h.lock_surface(1);
    let longest = "a".repeat(oxyos_protocol::MAX_SECRET_BYTES);
    assert_eq!(
        h.give_password(surface, &format!("{longest}a")),
        Err(ErrorCode::QuotaExceeded)
    );
    h.give_password(surface, &longest).unwrap();
    assert_eq!(
        h.scene.take_authentication().unwrap().1.len(),
        longest.len()
    );
}

#[test]
fn a_lock_surface_that_goes_takes_its_password_with_it() {
    let mut h = Harness::new();
    h.scene.lock();
    let surface = h.lock_surface(1);
    h.give_password(surface, "submitted").unwrap();
    // The lock screen crashes before the session took the password: it is dropped, and the next
    // lock screen's password is checked, not busy.
    h.scene.disconnect(LOCKER);
    assert!(h.scene.take_authentication().is_none());
    h.scene
        .connect(
            LOCKER,
            Grants {
                lock: true,
                ..Grants::default()
            },
        )
        .unwrap();
    let surface = h.lock_surface(2);
    h.give_password(surface, "again").unwrap();
    assert_eq!(h.scene.take_authentication().unwrap().0, surface);
    // A password being checked when its surface goes is still answered by the session, to no
    // one, and the next is checked.
    h.request(LOCKER, Request::Destroy { surface: 2 }).unwrap();
    h.scene
        .authenticated(surface, oxyos_protocol::AuthOutcome::Rejected, 0);
    let surface = h.lock_surface(3);
    h.give_password(surface, "third").unwrap();
    assert!(h.scene.take_authentication().is_some());
    assert!(h.results(LOCKER).is_empty());
}

#[test]
fn caps_lock_is_told_to_the_focused_surface_and_to_each_that_gains_focus() {
    let mut h = Harness::new();
    let first = h.toplevel(APP, 1, 400, 300);
    h.clear_log();
    h.scene.set_caps_lock(true);
    h.pump();
    assert_eq!(
        h.events_for(APP),
        vec![Event::CapsLock {
            surface: 1,
            on: true
        }]
    );
    assert!(h.events_for(OTHER).is_empty());
    // Telling again what is so tells nothing.
    h.clear_log();
    h.scene.set_caps_lock(true);
    h.pump();
    assert!(h.events_for(APP).is_empty());
    // A surface gaining focus while it is on is told after its focus.
    let second = h.toplevel(OTHER, 1, 400, 300);
    assert_eq!(h.scene.focus(), Some(second));
    let told: Vec<Event> = h
        .events_for(OTHER)
        .into_iter()
        .filter(|event| matches!(event, Event::Focus { .. } | Event::CapsLock { .. }))
        .collect();
    assert_eq!(
        told,
        vec![
            Event::Focus {
                surface: 1,
                focused: true
            },
            Event::CapsLock {
                surface: 1,
                on: true
            }
        ]
    );
    // Off: only the focused surface is told; the other learns it as it gains focus (nothing).
    h.clear_log();
    h.scene.set_caps_lock(false);
    h.pump();
    assert_eq!(
        h.events_for(OTHER),
        vec![Event::CapsLock {
            surface: 1,
            on: false
        }]
    );
    assert!(h.events_for(APP).is_empty());
    h.scene.activate(first);
    h.pump();
    assert!(!h
        .events_for(APP)
        .iter()
        .any(|event| matches!(event, Event::CapsLock { .. })));
}

#[test]
fn surfaces_arrive_with_their_first_frame_and_go_when_gone_not_when_hidden() {
    use super::appearances::{Appearance, Change, Kind};
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let with = |key, kind, change| Appearance { key, kind, change };
    let gone = |key, kind| with(key, kind, Change::Gone);
    let came = |key, kind| with(key, kind, Change::Arrived);
    // Its first frame, in the frame that composes it first.
    assert_eq!(h.scene.take_appearances(), vec![came(window, Kind::Window)]);
    // A popup arrives and, dismissed, goes.
    let w = h.rect(window);
    let popup = h.open_menu(
        window,
        2,
        (w.x + 10, w.y + 10),
        menu(Rect::new(0, 0, 40, 20), 200, 150),
    );
    assert_eq!(h.scene.take_appearances(), vec![came(popup, Kind::Popup)]);
    h.click(w.x + 380, w.y + 290);
    assert_eq!(h.scene.take_appearances(), vec![gone(popup, Kind::Popup)]);
    // Minimized it goes into the dock; restored it comes out of it.
    h.scene.minimize(window);
    assert_eq!(
        h.scene.take_appearances(),
        vec![with(window, Kind::Window, Change::Minimized)]
    );
    h.scene.restore(window);
    assert_eq!(
        h.scene.take_appearances(),
        vec![with(window, Kind::Window, Change::Restored)]
    );
    // A switch of workspace moves it aside: out to the left going to a later workspace, in from
    // the left coming back to an earlier one.
    h.request(SHELL, Request::AddWorkspace {}).unwrap();
    assert!(h.scene.take_appearances().is_empty());
    h.request(SHELL, Request::ActivateWorkspace { workspace: 1 })
        .unwrap();
    assert_eq!(
        h.scene.take_appearances(),
        vec![with(
            window,
            Kind::Window,
            Change::WorkspaceOut { forward: true }
        )]
    );
    h.request(SHELL, Request::ActivateWorkspace { workspace: 0 })
        .unwrap();
    assert_eq!(
        h.scene.take_appearances(),
        vec![with(
            window,
            Kind::Window,
            Change::WorkspaceIn { forward: false }
        )]
    );
    // Under the lock it neither arrives nor goes.
    h.scene.lock();
    let lock = h.lock_surface(1);
    assert_eq!(h.scene.take_appearances(), vec![came(lock, Kind::Lock)]);
    // Unlocked, the lock screen goes over the session shown again.
    h.scene.unlock();
    assert_eq!(h.scene.take_appearances(), vec![gone(lock, Kind::Lock)]);
    // Destroyed, a shown window goes; one never shown does not.
    h.request(APP, Request::Destroy { surface: 1 }).unwrap();
    assert_eq!(h.scene.take_appearances(), vec![gone(window, Kind::Window)]);
    h.request(
        APP,
        Request::CreateToplevel {
            surface: 3,
            size: Size::new(100, 100),
        },
    )
    .unwrap();
    h.request(APP, Request::Destroy { surface: 3 }).unwrap();
    assert!(h.scene.take_appearances().is_empty());
}

#[test]
fn a_reader_holds_only_the_lock_screen_while_locked() {
    let mut h = Harness::new();
    h.scene.connect(READER, reader()).unwrap();
    let window = h.toplevel(APP, 1, 400, 300);
    publish(&mut h, window, "Message actions");
    h.request(
        LOCKER,
        Request::CreateLockSurface {
            surface: 1,
            output: 1,
        },
    )
    .unwrap();
    let lock = key(LOCKER, 1);
    h.present(lock);
    h.clear_log();
    h.scene.lock();
    h.pump();
    // Locking takes the window's tree from the reader.
    assert!(h.events_for(READER).contains(&Event::AccessibilityRemoved {
        client: APP,
        surface: 1
    }));
    // While locked, what the window publishes does not reach it; the lock screen's does.
    h.clear_log();
    publish(&mut h, window, "Secret mail");
    assert!(!h.events_for(READER).iter().any(is_accessibility));
    publish(&mut h, lock, "Lock screen");
    assert_eq!(read_tree(&h, lock)[0].1, "Lock screen");
    // A reader that connects while locked is given the lock screen alone.
    h.scene.disconnect(READER);
    h.clear_log();
    h.scene.connect(READER, reader()).unwrap();
    h.pump();
    assert!(read_tree(&h, window).is_empty());
    assert_eq!(read_tree(&h, lock)[0].1, "Lock screen");
    // A window going away while locked tells the reader nothing: it holds no tree of it.
    let other = h.toplevel(OTHER, 1, 200, 100);
    publish(&mut h, other, "Other");
    h.clear_log();
    h.request(OTHER, Request::Destroy { surface: 1 }).unwrap();
    assert!(!h.events_for(READER).iter().any(is_accessibility));
    // Unlocked, it is given the window's tree as it is now.
    h.clear_log();
    h.scene.unlock();
    h.pump();
    assert_eq!(read_tree(&h, window)[0].1, "Secret mail");
}

/// A snapshot offering actions: an adjustable node with two custom actions, a text field with
/// its selection, and a plain button.
fn publish_actions(h: &mut Harness, surface: SurfaceKey) -> Result<(), ErrorCode> {
    use oxyos_protocol::AccessibleActions;
    let actions = |bits| AccessibleActions::new(bits).unwrap();
    let send = |h: &mut Harness, request| h.request(surface.client, request);
    send(
        h,
        Request::AccessibilityBegin {
            surface: surface.id,
            nodes: 3,
        },
    )?;
    for (index, role, bits) in [
        (
            0,
            AccessibleRole::Slider,
            AccessibleActions::INCREMENT | AccessibleActions::DECREMENT,
        ),
        (
            1,
            AccessibleRole::TextInput,
            AccessibleActions::SET_SELECTION,
        ),
        (2, AccessibleRole::Button, 0),
    ] {
        send(
            h,
            Request::AccessibleNode {
                surface: surface.id,
                index,
                parent: NO_ACCESSIBLE_PARENT,
                role,
                state: AccessibleState::default(),
                actions: actions(bits),
                rect: Rect::new(0, 40 * index as i32, 100, 40),
            },
        )?;
    }
    for (index, field, text) in [
        (0, AccessibleField::Name, "Volumen"),
        (0, AccessibleField::Actions, "Reiniciar\nCompartir"),
        (1, AccessibleField::Value, "hola ñandú"),
        (2, AccessibleField::Name, "Aceptar"),
    ] {
        for chunk in TextChunk::split(text) {
            send(
                h,
                Request::AccessibleText {
                    surface: surface.id,
                    index,
                    field,
                    text: chunk,
                },
            )?;
        }
    }
    send(
        h,
        Request::AccessibleSelection {
            surface: surface.id,
            index: 1,
            start: 2,
            end: 4,
        },
    )?;
    send(
        h,
        Request::AccessibilityCommit {
            surface: surface.id,
        },
    )
}

#[test]
fn an_assistive_technology_holds_what_nodes_offer_and_uses_only_that() {
    use oxyos_protocol::AccessibleActions;
    let mut h = Harness::new();
    h.scene.connect(READER, reader()).unwrap();
    let window = h.toplevel(APP, 1, 400, 300);
    h.clear_log();
    publish_actions(&mut h, window).unwrap();
    let held = h.events_for(READER);
    let actions_of = |index| {
        held.iter().find_map(|event| match event {
            Event::AccessibleNode {
                index: i, actions, ..
            } if *i == index => Some(actions.bits()),
            _ => None,
        })
    };
    assert_eq!(
        actions_of(0),
        Some(AccessibleActions::INCREMENT | AccessibleActions::DECREMENT)
    );
    assert_eq!(actions_of(2), Some(0));
    assert!(held.contains(&Event::AccessibleSelection {
        client: APP,
        surface: 1,
        index: 1,
        start: 2,
        end: 4,
    }));
    let labels: String = held
        .iter()
        .filter_map(|event| match event {
            Event::AccessibleText {
                index: 0,
                field: AccessibleField::Actions,
                text,
                ..
            } => Some(text.as_str().to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(labels, "Reiniciar\nCompartir");
    // What is offered reaches the application, with what it names; what is not, nothing.
    let act = |index, action, custom, start, end| Request::AccessibilityAction {
        client: APP,
        surface: 1,
        index,
        action,
        custom,
        start,
        end,
    };
    let delivered = |h: &mut Harness, request| {
        h.clear_log();
        h.request(READER, request).unwrap();
        h.events_for(APP).into_iter().find_map(|event| match event {
            Event::AccessibilityAction {
                index,
                action,
                custom,
                start,
                end,
                ..
            } => Some((index, action, custom, start, end)),
            _ => None,
        })
    };
    use AccessibleAction::*;
    assert_eq!(
        delivered(&mut h, act(0, Increment, 9, 9, 9)),
        Some((0, Increment, 0, 0, 0))
    );
    assert_eq!(
        delivered(&mut h, act(0, Decrement, 0, 0, 0)),
        Some((0, Decrement, 0, 0, 0))
    );
    assert_eq!(
        delivered(&mut h, act(0, Custom, 1, 0, 0)),
        Some((0, Custom, 1, 0, 0))
    );
    assert_eq!(
        delivered(&mut h, act(1, SetSelection, 0, 0, 7)),
        Some((1, SetSelection, 0, 0, 7))
    );
    for refused in [
        act(0, ScrollForward, 0, 0, 0),
        act(0, Dismiss, 0, 0, 0),
        act(0, Custom, 2, 0, 0),
        act(2, Increment, 0, 0, 0),
        act(2, Custom, 0, 0, 0),
        // Inside ñ, past the value, backwards.
        act(1, SetSelection, 0, 0, 6),
        act(1, SetSelection, 0, 0, 99),
        act(1, SetSelection, 0, 4, 2),
        act(0, SetSelection, 0, 0, 0),
    ] {
        assert_eq!(delivered(&mut h, refused), None, "{refused:?}");
    }
}

/// Whether a snapshot of one text field valued "hola ñandú" with this selection commits.
fn commit_selection(start: u32, end: u32) -> Result<(), ErrorCode> {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    h.request(
        APP,
        Request::AccessibilityBegin {
            surface: window.id,
            nodes: 1,
        },
    )?;
    h.request(
        APP,
        Request::AccessibleNode {
            surface: window.id,
            index: 0,
            parent: NO_ACCESSIBLE_PARENT,
            role: AccessibleRole::TextInput,
            state: AccessibleState::default(),
            actions: Default::default(),
            rect: Rect::new(0, 0, 100, 40),
        },
    )?;
    for chunk in TextChunk::split("hola ñandú") {
        h.request(
            APP,
            Request::AccessibleText {
                surface: window.id,
                index: 0,
                field: AccessibleField::Value,
                text: chunk,
            },
        )?;
    }
    h.request(
        APP,
        Request::AccessibleSelection {
            surface: window.id,
            index: 0,
            start,
            end,
        },
    )?;
    h.request(APP, Request::AccessibilityCommit { surface: window.id })
}

#[test]
fn a_selection_is_a_range_of_its_value() {
    assert_eq!(commit_selection(5, 7), Ok(()));
    assert_eq!(commit_selection(12, 12), Ok(()));
    assert_eq!(commit_selection(6, 7), Err(ErrorCode::InvalidMessage));
    assert_eq!(commit_selection(0, 13), Err(ErrorCode::InvalidMessage));
    assert_eq!(commit_selection(4, 2), Err(ErrorCode::InvalidMessage));
}

#[test]
fn announcements_reach_assistive_technologies_alone_and_not_from_behind_the_lock() {
    use oxyos_protocol::{Politeness, PreeditText};
    let mut h = Harness::new();
    h.scene.connect(READER, reader()).unwrap();
    let _window = h.toplevel(APP, 1, 400, 300);
    let announce = |h: &mut Harness, client, surface, text: &str, politeness| {
        let chunks = TextChunk::split(text);
        for (index, chunk) in chunks.iter().enumerate() {
            h.request(
                client,
                Request::AccessibilityAnnounce {
                    surface,
                    index: index as u8,
                    last: index + 1 == chunks.len(),
                    politeness,
                    text: PreeditText::new(chunk.as_str()).unwrap(),
                },
            )?;
        }
        Ok::<(), ErrorCode>(())
    };
    let heard = |h: &Harness| {
        let mut out: Vec<(u32, Politeness, String)> = Vec::new();
        for event in h.events_for(READER) {
            if let Event::AccessibilityAnnouncement {
                client,
                index,
                politeness,
                text,
                ..
            } = event
            {
                if index == 0 {
                    out.push((client, politeness, String::new()));
                }
                out.last_mut().unwrap().2.push_str(text.as_str());
            }
        }
        out
    };
    h.clear_log();
    let long = "Se ha guardado el documento «Informe anual» en la carpeta Documentos";
    announce(&mut h, APP, 1, "Guardado", Politeness::Polite).unwrap();
    announce(&mut h, APP, 1, long, Politeness::Assertive).unwrap();
    assert_eq!(
        heard(&h),
        vec![
            (APP, Politeness::Polite, "Guardado".to_string()),
            (APP, Politeness::Assertive, long.to_string()),
        ]
    );
    // No ordinary client hears them, the announcer included.
    for client in [APP, OTHER, SHELL] {
        assert!(!h
            .events_for(client)
            .iter()
            .any(|e| matches!(e, Event::AccessibilityAnnouncement { .. })));
    }
    // Chunks in order, within the limit, of one's own live surface.
    let chunk = |index, last| Request::AccessibilityAnnounce {
        surface: 1,
        index,
        last,
        politeness: Politeness::Polite,
        text: PreeditText::new("a").unwrap(),
    };
    assert_eq!(h.request(APP, chunk(1, true)), Err(ErrorCode::InvalidOrder));
    h.request(APP, chunk(0, false)).unwrap();
    assert_eq!(h.request(APP, chunk(2, true)), Err(ErrorCode::InvalidOrder));
    assert_eq!(
        announce(&mut h, APP, 9, "Nada", Politeness::Polite),
        Err(ErrorCode::UnknownSurface)
    );
    let too_long = "a".repeat(oxyos_protocol::MAX_ANNOUNCEMENT_BYTES + 1);
    assert_eq!(
        announce(&mut h, APP, 1, &too_long, Politeness::Polite),
        Err(ErrorCode::QuotaExceeded)
    );
    // Locked: what the session announces is not heard; the lock screen's is.
    h.scene.lock();
    let lock = h.lock_surface(1);
    h.clear_log();
    announce(&mut h, APP, 1, "Mensaje nuevo de Ana", Politeness::Polite).unwrap();
    announce(
        &mut h,
        LOCKER,
        lock.id,
        "Contraseña incorrecta",
        Politeness::Assertive,
    )
    .unwrap();
    assert_eq!(
        heard(&h),
        vec![(
            LOCKER,
            Politeness::Assertive,
            "Contraseña incorrecta".to_string()
        )]
    );
}

#[test]
fn the_dock_is_the_shells_bottom_panel() {
    let mut h = Harness::new();
    assert_eq!(h.scene.dock(), None);
    h.request(
        SHELL,
        Request::CreateShellSurface {
            surface: 1,
            kind: ShellKind::Panel,
            edge: PanelEdge::Top,
            output: 1,
            size: Size::new(1920, 28),
            exclusive: 28,
        },
    )
    .unwrap();
    h.present(key(SHELL, 1));
    assert_eq!(h.scene.dock(), None, "a top panel is no dock");
    h.request(
        SHELL,
        Request::CreateShellSurface {
            surface: 2,
            kind: ShellKind::Panel,
            edge: PanelEdge::Bottom,
            output: 1,
            size: Size::new(1920, 56),
            exclusive: 56,
        },
    )
    .unwrap();
    // Until it shows, it is no dock either.
    assert_eq!(h.scene.dock(), None);
    h.present(key(SHELL, 2));
    assert_eq!(h.scene.dock(), Some(Rect::new(0, 1080 - 56, 1920, 56)));
}

#[test]
fn the_shell_asks_for_an_application_by_id_and_is_answered() {
    use super::launch::LaunchRequest;
    let mut h = Harness::new();
    let chunk = |text: &str| PreeditText::new(text).unwrap();
    // Only the shell asks.
    assert_eq!(
        h.request(
            APP,
            Request::LaunchApplication {
                serial: 1,
                index: 0,
                last: true,
                text: chunk("so.oxy.allo"),
            },
        ),
        Err(ErrorCode::Unauthorized)
    );
    // An id longer than a chunk comes in order; the request is whole at its last chunk.
    let id = "so.oxy.an-application-with-a-rather-long-id";
    let chunks: Vec<&str> = vec![&id[..32], &id[32..]];
    for (index, part) in chunks.iter().enumerate() {
        h.request(
            SHELL,
            Request::LaunchApplication {
                serial: 7,
                index: index as u8,
                last: index == chunks.len() - 1,
                text: chunk(part),
            },
        )
        .unwrap();
    }
    assert_eq!(
        h.scene.take_launches(),
        vec![LaunchRequest {
            client: SHELL,
            serial: 7,
            id: id.to_string()
        }]
    );
    assert!(h.scene.take_launches().is_empty());
    // Out of order: a protocol error.
    assert_eq!(
        h.request(
            SHELL,
            Request::LaunchApplication {
                serial: 8,
                index: 1,
                last: true,
                text: chunk("x"),
            },
        ),
        Err(ErrorCode::InvalidOrder)
    );
    // The answer goes to the shell alone.
    h.clear_log();
    h.scene.launched(SHELL, 7, true);
    h.pump();
    assert_eq!(
        h.events_for(SHELL),
        vec![Event::LaunchResult {
            serial: 7,
            launched: true
        }]
    );
    assert!(h.events_for(APP).is_empty());
    // A shell that goes takes its unanswered requests with it.
    h.request(
        SHELL,
        Request::LaunchApplication {
            serial: 9,
            index: 0,
            last: true,
            text: chunk("so.oxy.allo"),
        },
    )
    .unwrap();
    h.scene.disconnect(SHELL);
    assert!(h.scene.take_launches().is_empty());
}

#[test]
fn a_launched_processs_windows_are_its_registry_applications() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    // The application claims an id.
    for (index, text) in ["so.oxy.inbox"].iter().enumerate() {
        h.request(
            APP,
            Request::SetAppId {
                surface: 1,
                index: index as u8,
                last: true,
                text: PreeditText::new(text).unwrap(),
            },
        )
        .unwrap();
    }
    let told = |h: &mut Harness| {
        let mut launched = None;
        let mut app = String::new();
        for event in h.events_for(SHELL) {
            match event {
                Event::WindowState { launched: l, .. } => launched = Some(l),
                Event::WindowAppId { index, text, .. } => {
                    if index == 0 {
                        app.clear();
                    }
                    app.push_str(text.as_str());
                }
                _ => {}
            }
        }
        (launched, app)
    };
    h.scene.activate(window);
    h.pump();
    // Not launched by the session: its own claim, marked so (the latest the shell was told).
    assert_eq!(told(&mut h), (Some(false), "so.oxy.inbox".to_string()));
    // Its launch record names it: the registry's id, whatever it claims, and marked launched.
    h.scene.set_application(APP, "so.oxy.allo");
    h.pump();
    assert_eq!(told(&mut h), (Some(true), "so.oxy.allo".to_string()));
    assert_eq!(h.scene.application(APP), Some("so.oxy.allo"));
    // Gone with its client.
    h.scene.disconnect(APP);
    assert_eq!(h.scene.application(APP), None);
}

// ---- the user's preferences (ADR 0019) --------------------------------------------------------

/// Settings, given its grant by its launch record after it connected.
const SETTINGS: ClientId = 9;

fn drawing(dark: bool, accent: &str) -> DrawingPreferences {
    DrawingPreferences {
        dark,
        reduce_motion: false,
        reduce_transparency: true,
        accent: PreeditText::new(accent).unwrap(),
    }
}

fn values(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

/// The preferences `client` was told, each `key=value` whole.
fn told_preferences(h: &Harness, client: ClientId) -> Vec<String> {
    let mut told = Vec::new();
    let mut text = String::new();
    for event in h.events_for(client) {
        if let Event::Preference {
            index,
            last,
            text: chunk,
        } = event
        {
            if index == 0 {
                text.clear();
            }
            text.push_str(chunk.as_str());
            if last {
                told.push(std::mem::take(&mut text));
            }
        }
    }
    told
}

fn set_preference(
    h: &mut Harness,
    client: ClientId,
    serial: u32,
    text: &str,
) -> Result<(), ErrorCode> {
    let chunks = TextChunk::split(text);
    let count = chunks.len();
    for (index, chunk) in chunks.iter().enumerate() {
        h.request(
            client,
            Request::SetPreference {
                serial,
                index: index as u8,
                last: index + 1 == count,
                text: PreeditText::new(chunk.as_str()).unwrap(),
            },
        )?;
    }
    Ok(())
}

#[test]
fn only_settings_changes_preferences_and_its_changes_are_handed_over_whole() {
    let mut h = Harness::new();
    // No client has the grant by connecting, nor by asking: the shell is refused as well.
    for client in [APP, SHELL] {
        assert_eq!(
            set_preference(&mut h, client, 1, "appearance=dark"),
            Err(ErrorCode::Unauthorized)
        );
    }
    assert!(h.scene.take_preference_changes().is_empty());
    h.scene.connect(SETTINGS, Grants::default()).unwrap();
    h.scene
        .grant(
            SETTINGS,
            Grants {
                settings: true,
                ..Grants::default()
            },
        )
        .unwrap();
    // Longer than a chunk, in order; whole at its last chunk.
    let long = format!("wallpaper={}", "a".repeat(40));
    set_preference(&mut h, SETTINGS, 7, &long).unwrap();
    set_preference(&mut h, SETTINGS, 8, "reduce_motion=true").unwrap();
    assert_eq!(
        h.scene.take_preference_changes(),
        vec![
            PreferenceChange {
                client: SETTINGS,
                serial: 7,
                key: "wallpaper".into(),
                value: "a".repeat(40),
            },
            PreferenceChange {
                client: SETTINGS,
                serial: 8,
                key: "reduce_motion".into(),
                value: "true".into(),
            },
        ]
    );
    assert!(h.scene.take_preference_changes().is_empty());
    // The answer goes to Settings alone.
    h.clear_log();
    h.scene.preference_set(SETTINGS, 7, true);
    h.pump();
    assert_eq!(
        h.events_for(SETTINGS),
        vec![Event::PreferenceResult {
            serial: 7,
            accepted: true
        }]
    );
    assert!(h.events_for(APP).is_empty());
    // Text that is no change is refused at once, with nothing handed over.
    h.clear_log();
    set_preference(&mut h, SETTINGS, 9, "appearance").unwrap();
    assert_eq!(
        h.events_for(SETTINGS),
        vec![Event::PreferenceResult {
            serial: 9,
            accepted: false
        }]
    );
    assert!(h.scene.take_preference_changes().is_empty());
    // Out of order, or too long: protocol errors.
    assert_eq!(
        h.request(
            SETTINGS,
            Request::SetPreference {
                serial: 10,
                index: 1,
                last: true,
                text: PreeditText::new("x").unwrap(),
            },
        ),
        Err(ErrorCode::InvalidOrder)
    );
    assert_eq!(
        set_preference(&mut h, SETTINGS, 11, &"k".repeat(MAX_PREFERENCE_BYTES + 1)),
        Err(ErrorCode::QuotaExceeded)
    );
    // Revoked, it asks no more; and a client that goes takes its unanswered changes.
    set_preference(&mut h, SETTINGS, 12, "clock_24h=false").unwrap();
    h.scene.revoke(
        SETTINGS,
        Grants {
            settings: true,
            ..Grants::default()
        },
    );
    assert!(h.scene.take_preference_changes().is_empty());
    assert_eq!(
        set_preference(&mut h, SETTINGS, 13, "clock_24h=false"),
        Err(ErrorCode::Unauthorized)
    );
}

#[test]
fn every_client_is_told_the_preferences_it_draws_with_and_settings_every_one() {
    let mut h = Harness::new();
    // Nothing until the program running the compositor says what they are.
    assert!(h
        .events_for(APP)
        .iter()
        .all(|event| !matches!(event, Event::Preferences { .. })));
    h.clear_log();
    let kept = values(&[
        ("appearance", "auto"),
        ("accent", "jade"),
        ("lock_after", "5"),
        ("wallpaper", ""),
    ]);
    h.scene.set_preferences(drawing(false, "jade"), &kept);
    h.pump();
    let preferences = Event::Preferences {
        dark: false,
        reduce_motion: false,
        reduce_transparency: true,
        accent: PreeditText::new("jade").unwrap(),
    };
    for client in [APP, OTHER, SHELL, LOCKER] {
        assert_eq!(h.events_for(client), vec![preferences], "{client}");
    }
    // A client connecting is told at once; Settings, given its grant, every preference as kept.
    h.clear_log();
    h.scene.connect(SETTINGS, Grants::default()).unwrap();
    h.pump();
    assert_eq!(h.events_for(SETTINGS), vec![preferences]);
    assert!(told_preferences(&h, SETTINGS).is_empty());
    h.scene
        .grant(
            SETTINGS,
            Grants {
                settings: true,
                ..Grants::default()
            },
        )
        .unwrap();
    h.pump();
    assert_eq!(
        told_preferences(&h, SETTINGS),
        [
            "appearance=auto",
            "accent=jade",
            "lock_after=5",
            "wallpaper="
        ]
    );
    // Told again, nothing changed: nothing is sent.
    h.clear_log();
    h.scene.set_preferences(drawing(false, "jade"), &kept);
    h.pump();
    assert!(h.log.is_empty(), "{:?}", h.log);
    // A preference that does not change how clients draw: Settings alone is told it.
    let kept = values(&[
        ("appearance", "auto"),
        ("accent", "jade"),
        ("lock_after", "15"),
        ("wallpaper", ""),
    ]);
    h.scene.set_preferences(drawing(false, "jade"), &kept);
    h.pump();
    assert_eq!(told_preferences(&h, SETTINGS), ["lock_after=15"]);
    assert!(h.events_for(APP).is_empty());
    // The sun went down: every client draws dark, Settings still sees `auto`.
    h.clear_log();
    h.scene.set_preferences(drawing(true, "jade"), &kept);
    h.pump();
    let dark = Event::Preferences {
        dark: true,
        reduce_motion: false,
        reduce_transparency: true,
        accent: PreeditText::new("jade").unwrap(),
    };
    for client in [APP, OTHER, SHELL, LOCKER, SETTINGS] {
        assert_eq!(h.events_for(client), vec![dark], "{client}");
    }
}

#[test]
fn a_grant_one_client_holds_at_a_time_is_refused_whole_while_another_holds_it() {
    let mut h = Harness::new();
    h.scene
        .grant(
            APP,
            Grants {
                input_method: true,
                ..Grants::default()
            },
        )
        .unwrap();
    assert_eq!(
        h.scene.grant(
            OTHER,
            Grants {
                input_method: true,
                settings: true,
                ..Grants::default()
            },
        ),
        Err(ErrorCode::Unauthorized)
    );
    // Nothing was given.
    assert_eq!(
        set_preference(&mut h, OTHER, 1, "appearance=dark"),
        Err(ErrorCode::Unauthorized)
    );
    // Granting what a client holds is nothing new; a client gone is given nothing.
    assert_eq!(
        h.scene.grant(
            APP,
            Grants {
                input_method: true,
                ..Grants::default()
            },
        ),
        Ok(())
    );
    assert_eq!(h.scene.grant(42, Grants::default()), Ok(()));
}

// ---- the screen reader's keys (ADR 0014) -----------------------------------------------------

/// The screen reader: an assistive technology whose keys are its own.
const SCREEN_READER: ClientId = 8;
const CTRL: u32 = 29;
const RIGHT_CTRL: u32 = 97;
const ALT: u32 = 56;
const ALT_GR: u32 = 100;
const SPACE: u32 = 57;
const LEFT: u32 = 105;
const RIGHT: u32 = 106;

fn screen_reader() -> Grants {
    Grants {
        accessibility: true,
        screen_reader: true,
        ..Grants::default()
    }
}

/// The keys `client` was sent, as (key, pressed).
fn keys_to(events: &[(ClientId, Event)], client: ClientId) -> Vec<(u32, bool)> {
    to(events, client)
        .iter()
        .filter_map(|event| match event {
            Event::Key { key, pressed, .. } => Some((*key, *pressed)),
            _ => None,
        })
        .collect()
}

fn commands(events: &[(ClientId, Event)]) -> Vec<ReaderCommand> {
    to(events, SCREEN_READER)
        .iter()
        .filter_map(|event| match event {
            Event::ScreenReaderCommand { command } => Some(*command),
            _ => None,
        })
        .collect()
}

/// Ctrl+Alt+`key`, pressed and released, through the keyboard's path.
fn chord(h: &mut Harness, key: u32, ctrl: u32) {
    h.scene.keyboard(ctrl, true, None);
    h.scene.keyboard(ALT, true, None);
    h.scene.keyboard(key, true, (key == SPACE).then_some(" "));
    h.scene.keyboard(key, false, None);
    h.scene.keyboard(ALT, false, None);
    h.scene.keyboard(ctrl, false, None);
}

#[test]
fn the_screen_readers_keys_reach_it_alone() {
    let mut h = Harness::new();
    h.scene.connect(SCREEN_READER, screen_reader()).unwrap();
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    logged(&mut h);
    chord(&mut h, RIGHT, CTRL);
    chord(&mut h, LEFT, RIGHT_CTRL);
    chord(&mut h, SPACE, CTRL);
    let events = logged(&mut h);
    assert_eq!(
        commands(&events),
        [
            ReaderCommand::Next,
            ReaderCommand::Previous,
            ReaderCommand::Activate
        ]
    );
    // The application sees the modifiers, never the chord's key, its release or its text.
    let modifiers = |ctrl| [(ctrl, true), (ALT, true), (ALT, false), (ctrl, false)];
    let expected = [modifiers(CTRL), modifiers(RIGHT_CTRL), modifiers(CTRL)].concat();
    assert_eq!(keys_to(&events, APP), expected);
    assert!(!to(&events, APP)
        .iter()
        .any(|event| matches!(event, Event::TextInputCommitText { .. })));
    // The same keys without both modifiers, or with AltGr (which types), are the application's.
    h.scene.keyboard(RIGHT, true, None);
    h.scene.keyboard(RIGHT, false, None);
    h.scene.keyboard(CTRL, true, None);
    h.scene.keyboard(ALT_GR, true, None);
    h.scene.keyboard(SPACE, true, None);
    h.scene.keyboard(SPACE, false, None);
    let events = logged(&mut h);
    assert!(commands(&events).is_empty());
    assert!(keys_to(&events, APP).contains(&(RIGHT, false)));
    assert!(keys_to(&events, APP).contains(&(SPACE, true)));
}

#[test]
fn without_a_screen_reader_its_keys_are_ordinary() {
    let mut h = Harness::new();
    // An assistive technology without the grant has no keys.
    h.scene.connect(READER, reader()).unwrap();
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    logged(&mut h);
    chord(&mut h, RIGHT, CTRL);
    let events = logged(&mut h);
    assert!(keys_to(&events, APP).contains(&(RIGHT, true)));
    assert!(keys_to(&events, APP).contains(&(RIGHT, false)));
    assert!(to(&events, READER).is_empty());
}

#[test]
fn the_screen_readers_keys_come_before_the_input_method_shortcuts_and_the_lock() {
    let (mut h, _) = served("");
    h.scene.connect(SCREEN_READER, screen_reader()).unwrap();
    // Even were Right a shortcut of the shell's.
    h.scene.set_shortcuts([SUPER, RIGHT]);
    h.scene.keyboard(CTRL, true, None);
    let ctrl = held(&logged(&mut h));
    h.scene.keyboard(ALT, true, None);
    let alt = held(&logged(&mut h));
    for serial in [ctrl, alt] {
        h.request(
            IME,
            Request::InputMethodKeyHandled {
                serial,
                used: false,
            },
        )
        .unwrap();
    }
    logged(&mut h);
    h.scene.keyboard(RIGHT, true, None);
    h.scene.keyboard(RIGHT, false, None);
    let events = logged(&mut h);
    assert_eq!(commands(&events), [ReaderCommand::Next]);
    assert!(to(&events, IME).is_empty());
    assert!(to(&events, SHELL).is_empty());
    assert!(to(&events, APP).is_empty());
    // Locked, the reader still has its keys: it reads the lock screen.
    h.scene.lock();
    logged(&mut h);
    h.scene.key(SPACE, true);
    h.scene.key(SPACE, false);
    assert_eq!(commands(&logged(&mut h)), [ReaderCommand::Activate]);
}

#[test]
fn one_screen_reader_and_a_release_is_taken_with_its_press() {
    let mut h = Harness::new();
    h.scene.connect(SCREEN_READER, screen_reader()).unwrap();
    assert_eq!(
        h.scene.connect(9, screen_reader()),
        Err(ErrorCode::Unauthorized)
    );
    let window = h.toplevel(APP, 1, 400, 300);
    h.activate(window);
    logged(&mut h);
    h.scene.key(CTRL, true);
    h.scene.key(ALT, true);
    h.scene.key(RIGHT, true);
    // The reader goes with the key held: its release is still not the application's.
    h.scene.disconnect(SCREEN_READER);
    h.scene.key(RIGHT, false);
    assert_eq!(keys_to(&logged(&mut h), APP), [(CTRL, true), (ALT, true)]);
    // With no reader, the chord is the application's again.
    h.scene.key(RIGHT, true);
    h.scene.key(RIGHT, false);
    assert_eq!(
        keys_to(&logged(&mut h), APP),
        [(RIGHT, true), (RIGHT, false)]
    );
    assert_eq!(h.scene.connect(9, screen_reader()), Ok(()));
}

// ---- a window's own operations (ADR 0016) ------------------------------------------------

fn activations(h: &Harness) -> Vec<(ClientId, u32, bool)> {
    h.log
        .iter()
        .filter_map(|(client, event)| match event {
            Event::Activated { surface, active } => Some((*client, *surface, *active)),
            _ => None,
        })
        .collect()
}

fn last_configure_maximized(h: &Harness, surface: SurfaceKey) -> Option<bool> {
    h.log.iter().rev().find_map(|(client, event)| match event {
        Event::Configure {
            surface: id,
            maximized,
            ..
        } if *client == surface.client && *id == surface.id => Some(*maximized),
        _ => None,
    })
}

#[test]
fn the_compositor_resizes_a_window_from_the_band_outside_its_edges() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let w = h.rect(window);
    // Outside the right edge: the resize pointer, and the window is not entered.
    h.clear_log();
    h.scene.pointer_motion(w.x + 400 + 3, w.y + 150);
    assert_eq!(h.scene.pointer_shape(), PointerShape::ResizeHorizontal);
    h.scene.pointer_motion(w.x - 3, w.y - 3);
    assert_eq!(h.scene.pointer_shape(), PointerShape::ResizeTopLeft);
    h.scene.pointer_motion(w.x + 400 + 2, w.y + 300 - 5);
    assert_eq!(
        h.scene.pointer_shape(),
        PointerShape::ResizeTopLeft,
        "the corner"
    );
    h.scene.pointer_motion(w.x + 200, w.y + 300 + 4);
    assert_eq!(h.scene.pointer_shape(), PointerShape::ResizeVertical);
    h.scene.pointer_motion(w.x + 200, w.y + 300 + 7);
    assert_eq!(
        h.scene.pointer_shape(),
        PointerShape::Arrow,
        "beyond the band"
    );

    // A drag from the right edge: the client is configured, and sees no press.
    h.scene.pointer_motion(w.x + 400 + 3, w.y + 150);
    h.scene.pointer_button(BUTTON, true);
    h.scene.pointer_motion(w.x + 400 + 53, w.y + 170);
    h.scene.pointer_button(BUTTON, false);
    h.pump();
    assert_eq!(pointer_presses(&h.events_for(APP)), 0);
    h.present(window);
    assert_eq!(h.rect(window), Rect::new(w.x, w.y, 450, 300));

    // From the left edge: the right edge stays where it is.
    h.scene.pointer_motion(w.x - 3, w.y + 150);
    h.scene.pointer_button(BUTTON, true);
    h.scene.pointer_motion(w.x - 43, w.y + 150);
    h.scene.pointer_button(BUTTON, false);
    h.present(window);
    assert_eq!(h.rect(window), Rect::new(w.x - 40, w.y, 490, 300));

    // Never outside the window's limits.
    h.request(
        APP,
        Request::SetSizeLimits {
            surface: 1,
            min_width: 380,
            min_height: 280,
            max_width: 500,
            max_height: 0,
        },
    )
    .unwrap();
    let w = h.rect(window);
    h.scene
        .pointer_motion(w.right() as i32 + 3, w.bottom() as i32 + 3);
    h.scene.pointer_button(BUTTON, true);
    h.scene
        .pointer_motion(w.right() as i32 + 203, w.bottom() as i32 + 203);
    h.scene.pointer_button(BUTTON, false);
    h.present(window);
    assert_eq!(h.rect(window), Rect::new(w.x, w.y, 500, 500));
    let w = h.rect(window);
    h.scene.pointer_motion(w.x - 3, w.y - 3);
    h.scene.pointer_button(BUTTON, true);
    h.scene.pointer_motion(w.x + 400, w.y + 400);
    h.scene.pointer_button(BUTTON, false);
    h.present(window);
    assert_eq!(
        h.rect(window),
        Rect::new(w.x + 120, w.y + 220, 380, 280),
        "the far corner stays"
    );
    // Limits a window is outside of configure it at once.
    h.request(
        APP,
        Request::SetSizeLimits {
            surface: 1,
            min_width: 600,
            min_height: 0,
            max_width: 0,
            max_height: 0,
        },
    )
    .unwrap();
    h.present(window);
    assert_eq!(h.rect(window).width, 600);
    assert_eq!(
        h.request(
            APP,
            Request::SetSizeLimits {
                surface: 1,
                min_width: 600,
                min_height: 0,
                max_width: 500,
                max_height: 0,
            },
        ),
        Err(ErrorCode::InvalidGeometry)
    );
}

/// The sizes `surface` was configured with since the log was cleared, in order.
fn configured_sizes(h: &Harness, surface: SurfaceKey) -> Vec<(u32, u32)> {
    h.log
        .iter()
        .filter_map(|(client, event)| match event {
            Event::Configure {
                surface: id, rect, ..
            } if *client == surface.client && *id == surface.id => Some((rect.width, rect.height)),
            _ => None,
        })
        .collect()
}

#[test]
fn an_edge_resize_keeps_one_configure_in_flight_and_the_latest_size_wins() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let w = h.rect(window);
    let edge = w.x + 400 + 3;
    h.clear_log();
    h.scene.pointer_motion(edge, w.y + 150);
    h.scene.pointer_button(BUTTON, true);
    // A hand's motions faster than the client draws: the first is configured, the others wait.
    for dx in [10, 20, 30, 40] {
        h.scene.pointer_motion(edge + dx, w.y + 150);
    }
    h.pump();
    assert_eq!(configured_sizes(&h, window), [(410, 300)]);
    assert_eq!(h.rect(window), w, "its last frame, where it was");
    // Its frame for that one is shown: the latest the pointer wanted goes, only it.
    h.present(window);
    assert_eq!(h.rect(window), Rect::new(w.x, w.y, 410, 300));
    assert_eq!(configured_sizes(&h, window), [(410, 300), (440, 300)]);
    h.scene.pointer_motion(edge + 50, w.y + 150);
    h.scene.pointer_motion(edge + 60, w.y + 150);
    h.scene.pointer_button(BUTTON, false);
    h.pump();
    assert_eq!(configured_sizes(&h, window).len(), 2, "still one in flight");
    // Its frame for that one shown, the size the pointer was released at goes.
    h.present(window);
    assert_eq!(h.rect(window), Rect::new(w.x, w.y, 440, 300));
    h.present(window);
    // Released, it ends exactly where the pointer did, and no configure follows.
    assert_eq!(h.rect(window), Rect::new(w.x, w.y, 460, 300));
    assert_eq!(
        configured_sizes(&h, window),
        [(410, 300), (440, 300), (460, 300)]
    );
    h.present(window);
    assert_eq!(configured_sizes(&h, window).len(), 3);
}

#[test]
fn a_resize_from_the_left_edge_keeps_the_right_edge_frame_by_frame() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let w = h.rect(window);
    let right = w.right();
    h.scene.pointer_motion(w.x - 3, w.y - 3);
    h.scene.pointer_button(BUTTON, true);
    // Every frame shown is where its own configure put it: the right and bottom edges stay.
    let mut shown = Vec::new();
    for step in 1..=12 {
        h.scene
            .pointer_motion(w.x - 3 - 7 * step, w.y - 3 - 5 * step);
        if step % 3 == 0 {
            h.present(window);
            shown.push(h.rect(window));
        }
    }
    h.scene.pointer_button(BUTTON, false);
    while h.scene.configure_serial(window)
        != h.scene.render_list().iter().find_map(|item| match item {
            RenderItem::Surface { key, frame, .. } if *key == window => Some(frame.configure),
            _ => None,
        })
    {
        h.present(window);
        shown.push(h.rect(window));
    }
    assert!(shown.len() >= 4, "{shown:?}");
    for rect in &shown {
        assert_eq!(
            (rect.right(), rect.bottom()),
            (right, w.bottom()),
            "{shown:?}"
        );
    }
    assert_eq!(
        *shown.last().unwrap(),
        Rect::new(w.x - 84, w.y - 60, 484, 360),
        "the pointer's"
    );
}

/// Every surface listed, displayed or being composed, is drawn at its buffer's size (scale 1).
fn assert_one_to_one(h: &Harness) {
    for (name, list) in [
        ("displayed", h.scene.render_list()),
        ("composed", h.scene.frame_list()),
    ] {
        for item in list {
            if let RenderItem::Surface { key, rect, frame } = item {
                assert_eq!(
                    (rect.width, rect.height),
                    (frame.extent.width, frame.extent.height),
                    "{key:?} drawn scaled ({name})"
                );
            }
        }
    }
}

#[test]
fn a_window_resizing_is_never_drawn_scaled() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let w = h.rect(window);
    h.scene.pointer_motion(w.x - 3, w.y + 150);
    h.scene.pointer_button(BUTTON, true);
    // A hand's drag from the left edge, the client answering every other motion: every frame
    // listed, displayed or being composed, draws the buffer at its own size.
    for step in 1..=20 {
        h.scene.pointer_motion(w.x - 3 - 6 * step, w.y + 150);
        assert_one_to_one(&h);
        if step % 2 == 0 {
            h.submit(window);
            h.scene.begin_frame();
            // A motion while its frame is on its way to the display (a flip pending).
            h.scene.pointer_motion(w.x - 6 - 6 * step, w.y + 150);
            assert_one_to_one(&h);
            h.scene.end_frame(1, Clock::Synthetic);
            h.pump();
            assert_one_to_one(&h);
        }
    }
    h.scene.pointer_button(BUTTON, false);
    // A configure that is not the drag's, while a frame answering the drag's is presenting: that
    // frame is shown at the size it was drawn for, where it was configured to be.
    h.submit(window);
    let answering = h.configures[&window].1;
    h.scene.begin_frame();
    h.scene.resize_window(window, Size::new(700, 500));
    h.pump();
    assert_one_to_one(&h);
    h.scene.end_frame(1, Clock::Synthetic);
    h.pump();
    assert_one_to_one(&h);
    let shown = h.rect(window);
    assert_eq!(
        (shown.width, shown.height),
        (answering.width, answering.height)
    );
    assert_eq!(shown.right(), w.right(), "where it was configured to be");
    h.present(window);
    assert_eq!(h.rect(window).width, 700);
    assert_one_to_one(&h);
}

#[test]
fn a_configure_of_the_size_shown_holds_no_resize() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let w = h.rect(window);
    // Configured again at its size (its output's scale set again): its client need not answer.
    h.scene.set_output_scale(1, 120).unwrap();
    h.clear_log();
    h.scene.pointer_motion(w.x + 403, w.y + 150);
    h.scene.pointer_button(BUTTON, true);
    h.scene.pointer_motion(w.x + 413, w.y + 150);
    h.pump();
    assert_eq!(configured_sizes(&h, window), [(410, 300)]);
}

#[test]
fn a_configure_of_its_own_replaces_what_a_resize_wanted() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    let w = h.rect(window);
    h.scene.pointer_motion(w.x + 403, w.y + 150);
    h.scene.pointer_button(BUTTON, true);
    h.scene.pointer_motion(w.x + 413, w.y + 150);
    h.scene.pointer_motion(w.x + 433, w.y + 150);
    h.scene.pointer_button(BUTTON, false);
    // The shell resizes it before the drag's latest size went: the shell's is the last word.
    h.scene.resize_window(window, Size::new(600, 400));
    h.clear_log();
    h.present(window);
    assert_eq!(h.rect(window), Rect::new(w.x, w.y, 600, 400));
    h.present(window);
    assert!(configured_sizes(&h, window).is_empty());
}

#[test]
fn the_resize_band_yields_to_what_is_above_it_and_is_absent_when_full_screen_or_locked() {
    let mut h = Harness::new();
    let below = h.toplevel(APP, 1, 400, 300);
    let above = h.toplevel(OTHER, 1, 800, 600);
    let w = h.rect(below);
    // The band of the window below is under the window above: the press is the window's.
    h.clear_log();
    h.click(w.x + 400 + 3, w.y + 150);
    assert_eq!(pointer_presses(&h.events_for(OTHER)), 2);
    assert_eq!(pointer_presses(&h.events_for(APP)), 0);
    h.request(OTHER, Request::Destroy { surface: 1 }).unwrap();
    let _ = above;

    h.request(
        APP,
        Request::SetFullscreen {
            surface: 1,
            enabled: true,
        },
    )
    .unwrap();
    h.present(below);
    h.scene.pointer_motion(10, 10);
    assert_eq!(h.scene.pointer_shape(), PointerShape::Arrow);
    h.request(
        APP,
        Request::SetFullscreen {
            surface: 1,
            enabled: false,
        },
    )
    .unwrap();
    h.present(below);
    h.scene.lock();
    h.pump();
    h.scene.pointer_motion(w.x + 400 + 3, w.y + 150);
    assert_eq!(h.scene.pointer_shape(), PointerShape::Arrow);
}

#[test]
fn a_window_zooms_to_the_work_area_and_restores_its_frame() {
    let mut h = Harness::new();
    h.request(
        SHELL,
        Request::CreateShellSurface {
            surface: 1,
            kind: ShellKind::Panel,
            edge: PanelEdge::Top,
            output: 1,
            size: Size::new(1, 32),
            exclusive: 32,
        },
    )
    .unwrap();
    h.present(key(SHELL, 1));
    let window = h.toplevel(APP, 1, 400, 300);
    let frame = h.rect(window);
    let zoom = |h: &mut Harness, enabled| {
        h.request(
            APP,
            Request::SetMaximized {
                surface: 1,
                enabled,
            },
        )
        .unwrap();
        h.present(window);
    };
    zoom(&mut h, true);
    assert_eq!(h.rect(window), Rect::new(0, 32, 1920, 1048));
    assert_eq!(last_configure_maximized(&h, window), Some(true));
    assert!(h.scene.is_maximized(window));
    zoom(&mut h, false);
    assert_eq!(h.rect(window), frame);
    assert_eq!(last_configure_maximized(&h, window), Some(false));

    // A window that cannot be as wide is centred in the work area at its widest.
    h.request(
        APP,
        Request::SetSizeLimits {
            surface: 1,
            min_width: 0,
            min_height: 0,
            max_width: 1000,
            max_height: 0,
        },
    )
    .unwrap();
    zoom(&mut h, true);
    assert_eq!(h.rect(window), Rect::new(460, 32, 1000, 1048));

    // Resizing it by hand ends the zoom: zooming again fills the work area, not restores.
    let w = h.rect(window);
    h.scene.pointer_motion(w.x - 3, w.y + 500);
    h.scene.pointer_button(BUTTON, true);
    h.scene.pointer_motion(w.x + 97, w.y + 500);
    h.scene.pointer_button(BUTTON, false);
    h.present(window);
    assert_eq!(last_configure_maximized(&h, window), Some(false));
    assert!(!h.scene.is_maximized(window));
    zoom(&mut h, true);
    assert_eq!(h.rect(window), Rect::new(460, 32, 1000, 1048));

    // Popups and other roles cannot zoom or minimize.
    let popup = h.popup(window, 2, menu(Rect::new(0, 0, 10, 10), 100, 100));
    h.present(popup);
    assert_eq!(
        h.request(
            APP,
            Request::SetMaximized {
                surface: 2,
                enabled: true,
            },
        ),
        Err(ErrorCode::InvalidRole)
    );
    assert_eq!(
        h.request(APP, Request::Minimize { surface: 2 }),
        Err(ErrorCode::InvalidRole)
    );
}

#[test]
fn a_window_minimizes_itself_as_the_shell_minimizes_it() {
    let mut h = Harness::new();
    let window = h.toplevel(APP, 1, 400, 300);
    h.request(APP, Request::Minimize { surface: 1 }).unwrap();
    h.frame();
    assert!(!h.rendered(window));
    assert!(h
        .scene
        .take_appearances()
        .iter()
        .any(|a| a.key == window && a.change == AppearanceChange::Minimized));
    // Restored by the shell, then locked: a window cannot minimize itself under the lock.
    h.scene.restore(window);
    h.frame();
    h.scene.lock();
    h.request(APP, Request::Minimize { surface: 1 }).unwrap();
    h.scene.unlock();
    h.frame();
    assert!(h.rendered(window));
}

#[test]
fn the_active_window_follows_focus_through_popups_and_panels_but_not_overlays_or_the_lock() {
    let mut h = Harness::new();
    let first = h.toplevel(APP, 1, 400, 300);
    assert_eq!(activations(&h), vec![(APP, 1, true)]);
    let second = h.toplevel(OTHER, 1, 300, 200);
    assert_eq!(h.scene.active_window(), Some(second));
    assert_eq!(
        activations(&h)[1..],
        [(APP, 1, false), (OTHER, 1, true)],
        "the old one first"
    );
    // A menu of the active window keeps it active.
    h.clear_log();
    let s = h.rect(second);
    h.open_menu(
        second,
        2,
        (s.x + 10, s.y + 10),
        menu(Rect::new(0, 0, 10, 10), 100, 100),
    );
    assert_eq!(activations(&h), vec![]);
    h.request(OTHER, Request::Destroy { surface: 2 }).unwrap();

    // The shell's panel taking focus leaves it active; its overlay does not.
    for (id, kind) in [(1, ShellKind::Panel), (2, ShellKind::Overlay)] {
        h.request(
            SHELL,
            Request::CreateShellSurface {
                surface: id,
                kind,
                edge: PanelEdge::Top,
                output: 1,
                size: Size::new(1, 32),
                exclusive: 0,
            },
        )
        .unwrap();
    }
    h.present(key(SHELL, 1));
    h.clear_log();
    h.click(10, 10);
    assert_eq!(h.scene.focus(), Some(key(SHELL, 1)));
    assert_eq!(h.scene.active_window(), Some(second));
    assert_eq!(activations(&h), vec![]);
    h.present(key(SHELL, 2));
    h.click(10, 10);
    assert_eq!(h.scene.focus(), Some(key(SHELL, 2)));
    assert_eq!(activations(&h), vec![(OTHER, 1, false)]);
    h.request(SHELL, Request::Destroy { surface: 2 }).unwrap();

    // Pressing a window makes it active; the lock leaves none, and unlocking gives it back.
    h.clear_log();
    let f = h.rect(first);
    h.click(f.x + 5, f.y + 5);
    assert_eq!(h.scene.active_window(), Some(first));
    h.scene.lock();
    h.pump();
    assert_eq!(h.scene.active_window(), None);
    h.scene.unlock();
    h.pump();
    assert_eq!(h.scene.active_window(), Some(first));
    assert_eq!(
        activations(&h),
        vec![
            (OTHER, 1, false),
            (APP, 1, true),
            (APP, 1, false),
            (APP, 1, true)
        ]
    );

    // The active window going makes the next one active, and the one gone is not told.
    h.clear_log();
    h.request(APP, Request::Destroy { surface: 1 }).unwrap();
    assert_eq!(h.scene.active_window(), Some(second));
    assert_eq!(activations(&h), vec![(OTHER, 1, true)]);
}

#[test]
fn windows_are_drawn_as_windows_and_the_active_one_so() {
    let mut h = Harness::new();
    let active = Some(WindowLook { active: true });
    let inactive = Some(WindowLook { active: false });
    let window = h.toplevel(APP, 1, 800, 600);
    let other = h.toplevel(OTHER, 1, 400, 300);
    assert_eq!(h.scene.focus(), Some(other));
    assert_eq!(
        (h.scene.window_look(window), h.scene.window_look(other)),
        (inactive, active)
    );
    h.scene.activate(window);
    assert_eq!(
        (h.scene.window_look(window), h.scene.window_look(other)),
        (active, inactive)
    );
    // A dialog is one, active with the focus, and so is its window, through it.
    h.request(
        APP,
        Request::CreateDialog {
            surface: 2,
            parent: 1,
            size: Size::new(300, 200),
        },
    )
    .unwrap();
    let dialog = key(APP, 2);
    h.present(dialog);
    assert_eq!(h.scene.focus(), Some(dialog));
    assert_eq!(
        (h.scene.window_look(dialog), h.scene.window_look(window)),
        (active, active)
    );
    assert_eq!(h.scene.window_look(other), inactive);
    h.request(APP, Request::Destroy { surface: 2 }).unwrap();
    assert_eq!(h.scene.focus(), Some(window));
    // A menu is no window; its window stays active while it has the focus.
    let w = h.rect(window);
    let popup = h.open_menu(
        window,
        3,
        (w.x + 10, w.y + 10),
        menu(Rect::new(10, 10, 20, 20), 100, 80),
    );
    assert_eq!(h.scene.focus(), Some(popup));
    assert_eq!(
        (h.scene.window_look(popup), h.scene.window_look(window)),
        (None, active)
    );
    // The shell's surfaces are not windows.
    h.request(
        SHELL,
        Request::CreateShellSurface {
            surface: 1,
            kind: ShellKind::Panel,
            edge: PanelEdge::Top,
            output: 1,
            size: Size::new(1, 40),
            exclusive: 32,
        },
    )
    .unwrap();
    h.present(key(SHELL, 1));
    assert_eq!(h.scene.window_look(key(SHELL, 1)), None);
    // Full-screen, from the frame that draws it so: before, it is still drawn as a window.
    h.request(
        OTHER,
        Request::SetFullscreen {
            surface: 1,
            enabled: true,
        },
    )
    .unwrap();
    h.frame();
    assert_eq!(h.scene.window_look(other), inactive);
    h.submit(other);
    h.scene.begin_frame();
    assert_eq!(h.scene.window_look(other), None, "the frame composed");
    h.scene.end_frame(1, Clock::Synthetic);
    assert_eq!(h.scene.window_look(other), None);
    // Back from it, a window again from its windowed frame.
    h.request(
        OTHER,
        Request::SetFullscreen {
            surface: 1,
            enabled: false,
        },
    )
    .unwrap();
    h.frame();
    assert_eq!(h.scene.window_look(other), None);
    h.present(other);
    assert!(h.scene.window_look(other).is_some());
    // The active window is the one told it is (`active_window`): locked, none is.
    assert_eq!(h.scene.active_window(), Some(window));
    h.scene.lock();
    assert_eq!(h.scene.active_window(), None);
    assert_eq!(
        (h.scene.window_look(window), h.scene.window_look(other)),
        (inactive, inactive)
    );
}

#[test]
fn new_windows_cascade_instead_of_opening_exactly_over_another() {
    let mut h = Harness::new();
    let first = h.toplevel(APP, 1, 400, 300);
    let second = h.toplevel(OTHER, 1, 400, 300);
    let third = h.toplevel(APP, 2, 400, 300);
    let (a, b, c) = (h.rect(first), h.rect(second), h.rect(third));
    assert_eq!((b.x - a.x, b.y - a.y), (24, 24));
    assert_eq!((c.x - a.x, c.y - a.y), (48, 48));
    // A window of another size is centred as before: nothing sits there.
    let other = h.toplevel(OTHER, 2, 200, 100);
    let o = h.rect(other);
    assert_eq!((o.x, o.y), ((1920 - 200) / 2, (1080 - 100) / 2));
    // One that would leave the work area starts again from its top-left corner.
    let mut h = Harness::new();
    let big = h.toplevel(APP, 1, 1900, 1070);
    let next = h.toplevel(OTHER, 1, 1900, 1070);
    assert_ne!(h.rect(big), h.rect(next));
    assert_eq!((h.rect(next).x, h.rect(next).y), (0, 0));
}

/// The menu bar (ADR 0021).
mod menus;
