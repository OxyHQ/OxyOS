use super::*;
use oxy_protocol::{Adjust, Anchor, Gravity};
use std::collections::HashMap;

const APP: ClientId = 1;
const OTHER: ClientId = 2;
const SHELL: ClientId = 3;
const LOCKER: ClientId = 4;
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
                    lock: false,
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
                format: oxy_protocol::Format::Xrgb8888,
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
    far.offset_x = oxy_protocol::MAX_COORDINATE;
    let popup = h.popup(window, 2, far);
    assert_eq!(h.dismissed(popup), Some(DismissReason::Unplaceable));
    // Every event the scene produced can be encoded.
    for (_, event) in &h.log {
        let mut bytes = Vec::new();
        oxy_protocol::write_event(&mut bytes, event).unwrap();
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
        format: oxy_protocol::Format::Argb8888,
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
                format: oxy_protocol::Format::Xrgb8888,
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
    // Too long, a cursor past the end or inside a character: nothing is delivered.
    assert!(!h.scene.preedit(&"a".repeat(33), 0));
    assert!(!h.scene.preedit("ab", 3));
    assert!(!h.scene.preedit("に", 1));
    assert!(h.pump().is_empty());
}
