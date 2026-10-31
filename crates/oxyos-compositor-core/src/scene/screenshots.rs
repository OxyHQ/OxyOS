//! Screenshots (docs/adr/0024-system-shortcuts.mdx, decision 10), as a Mac takes them:
//! Super+Shift+3 captures every display, each its own file; Super+Shift+4 opens the crosshair on
//! every display's shell, Space switching it to picking a window; Super+Shift+5 opens the toolbar
//! on the shell of the display in front. The compositor captures and the session writes the file;
//! the shell only shows the interface, and asks for the capture the person chose in it, citing the
//! serial of the interface it was told to show.
//!
//! While the interface is open the keyboard is the system's: Escape ends it, Space switches the
//! crosshair to a window and back, Return captures what the toolbar shows, and nothing else typed
//! reaches anybody, the active application keeping the keyboard focus, as during a Mac's ⌘⇧4.
//! The shell is told the window under the pointer as it moves (`CaptureTarget`), so it can show
//! the one a press would take. Locked, nothing is captured: the shortcuts do nothing (decision 7),
//! the lock ends an open interface and drops a capture not yet composed.
//!
//! What is never captured: a shell surface the shell leaves out (its screenshot interface, the
//! thumbnail), the lock screen, and every surface's secure regions (its secure text fields),
//! which a capture draws opaque black. Secure regions go with a surface's frame, as window views
//! do: sent with a Submit, shown when that frame presents.

use super::{ClientId, OutputId, Role, Scene, SurfaceKey, RECENT_INPUTS};
use oxyos_protocol::{
    CaptureDestination, CaptureKind, CaptureMode, ErrorCode, Event, PreeditText, Rect, Request,
    TextChunk, MAX_SCREENSHOT_PATH_BYTES, MAX_SECURE_REGIONS,
};
use std::collections::{BTreeMap, BTreeSet};

/// Captures asked for and not yet taken by the compositor: a person cannot ask for more between
/// two frames.
const MAX_PENDING_CAPTURES: usize = 8;
/// Screenshots the shell asked to open and the session has not taken.
const MAX_PENDING_OPENS: usize = 8;

/// A capture the compositor composes at its next frame: every display (`Screens`), a rectangle of
/// the global space on one display (`Area`, logical), or a window with its shadow (`Window`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureRequest {
    Screens {
        destination: CaptureDestination,
    },
    Area {
        output: OutputId,
        rect: Rect,
        destination: CaptureDestination,
    },
    Window {
        window: SurfaceKey,
        destination: CaptureDestination,
    },
}

impl CaptureRequest {
    pub fn destination(&self) -> CaptureDestination {
        match *self {
            CaptureRequest::Screens { destination }
            | CaptureRequest::Area { destination, .. }
            | CaptureRequest::Window { destination, .. } => destination,
        }
    }
}

/// The screenshot interface open on the shells: its serial (one for every shell and every change),
/// what it shows now, the output it was opened for, and the shells showing it.
#[derive(Clone, Debug)]
struct Interface {
    serial: u64,
    mode: CaptureMode,
    output: OutputId,
    shells: BTreeSet<ClientId>,
}

/// A surface's secure regions: the list being made, the one sent with a frame, the one shown.
#[derive(Debug, Default)]
struct SecureRegions {
    making: Option<Vec<Rect>>,
    sent: Option<(u64, Vec<Rect>)>,
    shown: Vec<Rect>,
}

#[derive(Debug)]
pub(super) struct Screenshots {
    interface: Option<Interface>,
    /// Where captures are saved: the toolbar's choice, kept for the session.
    destination: CaptureDestination,
    captures: Vec<CaptureRequest>,
    /// Shell surfaces left out of captures.
    excluded: BTreeSet<SurfaceKey>,
    secure: BTreeMap<SurfaceKey, SecureRegions>,
    /// The window under the pointer the shells were last told, by number, and its rectangle
    /// (none: there is none); `None` until they are told one.
    target: Option<Option<(u32, Rect)>>,
    opens: Vec<u32>,
}

impl Default for Screenshots {
    fn default() -> Self {
        Self {
            interface: None,
            destination: CaptureDestination::Desktop,
            captures: Vec::new(),
            excluded: BTreeSet::new(),
            secure: BTreeMap::new(),
            target: None,
            opens: Vec::new(),
        }
    }
}

/// The part of `rect` within `area`, if any.
fn within(rect: Rect, area: Rect) -> Option<Rect> {
    super::intersect(&rect, &area)
}

impl Scene {
    // ---- the shortcuts and the interface's keys -------------------------------------------

    /// Super+Shift+3: every display, at once; an interface open ends first.
    pub(super) fn capture_screens(&mut self) {
        self.end_capture_interface();
        let destination = self.screenshots.destination;
        self.push_capture(CaptureRequest::Screens { destination });
    }

    /// Super+Shift+4 (the crosshair, on every display's shell) or Super+Shift+5 (the toolbar, on
    /// the shell of the display in front). An interface already open ends first.
    pub(super) fn open_capture_interface(&mut self, mode: CaptureMode) {
        self.end_capture_interface();
        let output = self.shortcut_output();
        let shells: BTreeSet<ClientId> = match mode {
            CaptureMode::Selection => self.capture_shells(),
            _ => self.shortcut_shell(output).into_iter().collect(),
        };
        if shells.is_empty() {
            return;
        }
        let serial = self.next_serial();
        self.last_user_serial = serial;
        for &shell in &shells {
            self.credit_input(shell, serial);
        }
        self.screenshots.interface = Some(Interface {
            serial,
            mode,
            output,
            shells,
        });
        self.tell_capture_mode(mode);
        self.screenshots.target = None;
        self.follow_capture_target();
    }

    /// Every shell client granted `shortcuts` that has a shell surface: one for each display.
    fn capture_shells(&self) -> BTreeSet<ClientId> {
        self.nodes
            .iter()
            .filter(|(key, node)| {
                !node.dismissed
                    && matches!(node.role, Role::Shell { .. })
                    && self
                        .clients
                        .get(&key.client)
                        .is_some_and(|grants| grants.shell && grants.shortcuts)
            })
            .map(|(key, _)| key.client)
            .collect()
    }

    /// A key pressed while the interface is open, without Super: Escape ends it, Space switches
    /// the crosshair to picking a window and back, Return captures what the toolbar shows.
    /// Anything else does nothing, and reaches nobody.
    pub(super) fn capture_key(&mut self, key: u32) {
        const ESCAPE: u32 = 1;
        const ENTER: u32 = 28;
        const SPACE: u32 = 57;
        const KEYPAD_ENTER: u32 = 96;
        let Some(mode) = self.screenshots.interface.as_ref().map(|i| i.mode) else {
            return;
        };
        match (key, mode) {
            (ESCAPE, _) => self.end_capture_interface(),
            (SPACE, CaptureMode::Selection) => self.change_capture_mode(CaptureMode::Window),
            (SPACE, CaptureMode::Window) => self.change_capture_mode(CaptureMode::Selection),
            (ENTER | KEYPAD_ENTER, CaptureMode::Toolbar) => {
                // The toolbar stays the interface's mode: Confirm is a moment, not a state.
                self.tell_capture_mode(CaptureMode::Confirm);
            }
            _ => {}
        }
    }

    fn change_capture_mode(&mut self, mode: CaptureMode) {
        if let Some(interface) = &mut self.screenshots.interface {
            interface.mode = mode;
        }
        self.tell_capture_mode(mode);
    }

    /// Tells every shell showing the interface what it shows now.
    fn tell_capture_mode(&mut self, mode: CaptureMode) {
        let Some(interface) = self.screenshots.interface.clone() else {
            return;
        };
        for shell in interface.shells {
            if self.clients.contains_key(&shell) {
                self.emit(
                    shell,
                    Event::CaptureMode {
                        mode,
                        serial: interface.serial,
                        output: interface.output,
                    },
                );
            }
        }
    }

    /// Ends the interface, if open: every shell showing it is told.
    pub(super) fn end_capture_interface(&mut self) {
        if self.screenshots.interface.is_some() {
            self.tell_capture_mode(CaptureMode::Ended);
            self.screenshots.interface = None;
            self.screenshots.target = None;
        }
    }

    /// Whether the screenshot interface is open (from Super+Shift+4 or 5 until it ends).
    pub fn capturing(&self) -> bool {
        self.screenshots.interface.is_some()
    }

    /// The session locked: the interface ends and a capture not yet composed is dropped.
    pub(super) fn lock_screenshots(&mut self) {
        self.end_capture_interface();
        self.screenshots.captures.clear();
    }

    /// A shell gone, or no longer one: it no longer shows the interface, which ends with the last.
    pub(super) fn forget_capture_shell(&mut self, client: ClientId) {
        let Some(interface) = &mut self.screenshots.interface else {
            return;
        };
        interface.shells.remove(&client);
        if interface.shells.is_empty() {
            self.screenshots.interface = None;
            self.screenshots.target = None;
        }
    }

    /// Records `serial` as a user input of `client`'s, which it may cite.
    pub(super) fn credit_input(&mut self, client: ClientId, serial: u64) {
        let inputs = self.inputs.entry(client).or_default();
        inputs.push_back(serial);
        if inputs.len() > RECENT_INPUTS {
            inputs.pop_front();
        }
        self.activation.insert(client, serial);
    }

    // ---- the window under the pointer ---------------------------------------------------

    /// The window under the pointer, as a window pick would take it: the frontmost shown window
    /// whose rectangle holds the pointer, whatever shell surface is above it.
    fn window_under_pointer(&self) -> Option<(u32, Rect)> {
        let (x, y) = self.pointer;
        self.roots.iter().rev().find_map(|&root| {
            let node = self.nodes.get(&root)?;
            if node.role != Role::Toplevel || !self.visible(root) {
                return None;
            }
            let rect = self.global_rect(root)?;
            if !rect.contains_point(x, y) {
                return None;
            }
            Some((self.window_number(root)?, rect))
        })
    }

    /// While the interface is open, tells its shells the window under the pointer when it changed.
    pub(super) fn follow_capture_target(&mut self) {
        let Some(interface) = self.screenshots.interface.clone() else {
            return;
        };
        let target = self.window_under_pointer();
        if self.screenshots.target == Some(target) {
            return;
        }
        self.screenshots.target = Some(target);
        let (window, rect) = target.unwrap_or((0, Rect::new(0, 0, 0, 0)));
        for shell in interface.shells {
            if self.clients.contains_key(&shell) {
                self.emit(shell, Event::CaptureTarget { window, rect });
            }
        }
    }

    // ---- requests -----------------------------------------------------------------------

    pub(super) fn screenshot_request(
        &mut self,
        client: ClientId,
        request: Request,
    ) -> Result<(), ErrorCode> {
        let grants = *self.clients.get(&client).ok_or(ErrorCode::InvalidOrder)?;
        match request {
            Request::Capture {
                serial,
                kind,
                output,
                rect,
                window,
                destination,
            } => {
                if !(grants.shell && grants.shortcuts) {
                    return Err(ErrorCode::Unauthorized);
                }
                if !self.cites_interface(client, serial) {
                    return Ok(());
                }
                let capture = match kind {
                    CaptureKind::Screens => Some(CaptureRequest::Screens { destination }),
                    CaptureKind::Area => self
                        .outputs
                        .iter()
                        .find(|o| o.id == output)
                        .and_then(|o| within(rect, o.rect))
                        .map(|rect| CaptureRequest::Area {
                            output,
                            rect,
                            destination,
                        }),
                    CaptureKind::Window => self
                        .window_by_number(window)
                        .filter(|key| self.visible(*key))
                        .map(|window| CaptureRequest::Window {
                            window,
                            destination,
                        }),
                };
                // Nothing there to capture (a window gone meanwhile, a rectangle off the
                // display): the interface stays, for another choice.
                let Some(capture) = capture else {
                    return Ok(());
                };
                self.screenshots.destination = destination;
                self.end_capture_interface();
                self.push_capture(capture);
                Ok(())
            }
            Request::EndCapture { serial } => {
                if !(grants.shell && grants.shortcuts) {
                    return Err(ErrorCode::Unauthorized);
                }
                if self.cites_interface(client, serial) {
                    self.end_capture_interface();
                }
                Ok(())
            }
            Request::ExcludeFromCapture { surface, excluded } => {
                if !grants.shell {
                    return Err(ErrorCode::Unauthorized);
                }
                let key = SurfaceKey {
                    client,
                    id: surface,
                };
                let node = self.live(key).ok_or(ErrorCode::UnknownSurface)?;
                if !matches!(node.role, Role::Shell { .. }) {
                    return Err(ErrorCode::InvalidRole);
                }
                if excluded {
                    self.screenshots.excluded.insert(key);
                } else {
                    self.screenshots.excluded.remove(&key);
                }
                Ok(())
            }
            Request::ClearSecureRegions { surface } | Request::AddSecureRegion { surface, .. } => {
                let key = SurfaceKey {
                    client,
                    id: surface,
                };
                self.live(key).ok_or(ErrorCode::UnknownSurface)?;
                let regions = self.screenshots.secure.entry(key).or_default();
                match request {
                    Request::ClearSecureRegions { .. } => regions.making = Some(Vec::new()),
                    Request::AddSecureRegion { rect, .. } => {
                        let making = regions.making.as_mut().ok_or(ErrorCode::InvalidOrder)?;
                        if making.len() >= MAX_SECURE_REGIONS {
                            return Err(ErrorCode::QuotaExceeded);
                        }
                        if !rect.is_empty() {
                            making.push(rect);
                        }
                    }
                    _ => unreachable!(),
                }
                Ok(())
            }
            Request::OpenScreenshot { screenshot, serial } => {
                if !grants.shell {
                    return Err(ErrorCode::Unauthorized);
                }
                let recent = self
                    .inputs
                    .get(&client)
                    .is_some_and(|inputs| inputs.contains(&serial));
                if recent && !self.locked && self.screenshots.opens.len() < MAX_PENDING_OPENS {
                    self.screenshots.opens.push(screenshot);
                }
                Ok(())
            }
            _ => unreachable!("only screenshot requests are passed here"),
        }
    }

    /// Whether `client` is a shell showing the interface open with `serial`.
    fn cites_interface(&self, client: ClientId, serial: u64) -> bool {
        !self.locked
            && self
                .screenshots
                .interface
                .as_ref()
                .is_some_and(|i| i.serial == serial && i.shells.contains(&client))
    }

    fn push_capture(&mut self, capture: CaptureRequest) {
        if !self.locked && self.screenshots.captures.len() < MAX_PENDING_CAPTURES {
            self.screenshots.captures.push(capture);
        }
    }

    // ---- secure regions and what is left out ----------------------------------------------

    /// A frame of `surface` was submitted: the regions made since ClearSecureRegions go with it.
    pub(super) fn send_secure_regions(&mut self, surface: SurfaceKey, sequence: u64) {
        if let Some(regions) = self.screenshots.secure.get_mut(&surface) {
            if let Some(making) = regions.making.take() {
                regions.sent = Some((sequence, making));
            }
        }
    }

    /// A frame of `surface` began presenting: the regions sent with it, or an earlier one, are
    /// its regions from now on.
    pub(super) fn present_secure_regions(&mut self, surface: SurfaceKey, sequence: u64) {
        if let Some(regions) = self.screenshots.secure.get_mut(&surface) {
            if regions
                .sent
                .as_ref()
                .is_some_and(|(sent, _)| *sent <= sequence)
            {
                regions.shown = regions.sent.take().expect("checked").1;
            }
        }
    }

    pub(super) fn forget_screenshot_surface(&mut self, surface: SurfaceKey) {
        self.screenshots.secure.remove(&surface);
        self.screenshots.excluded.remove(&surface);
    }

    // ---- what the compositor and the session take -----------------------------------------

    /// The captures asked for since the last call, in order, for the compositor to compose at its
    /// next frame. None while locked.
    pub fn take_captures(&mut self) -> Vec<CaptureRequest> {
        if self.locked {
            self.screenshots.captures.clear();
        }
        std::mem::take(&mut self.screenshots.captures)
    }

    /// Whether a capture shows `key`: not a lock surface, nor a shell surface the shell left out.
    pub fn captured(&self, key: SurfaceKey) -> bool {
        !self.screenshots.excluded.contains(&key)
            && !self
                .nodes
                .get(&key)
                .is_some_and(|node| matches!(node.role, Role::Lock { .. }))
    }

    /// The secure regions of the frame `key` shows, surface-local and logical.
    pub fn secure_regions(&self, key: SurfaceKey) -> &[Rect] {
        self.screenshots
            .secure
            .get(&key)
            .map_or(&[], |regions| regions.shown.as_slice())
    }

    /// Whether `key` is `window` or drawn as part of it: one of its popups or dialogs.
    pub fn part_of_window(&self, key: SurfaceKey, window: SurfaceKey) -> bool {
        let mut current = Some(key);
        while let Some(at) = current {
            if at == window {
                return true;
            }
            current = self.nodes.get(&at).and_then(|node| node.parent);
        }
        false
    }

    /// The output a surface is on.
    pub fn output_of(&self, key: SurfaceKey) -> Option<OutputId> {
        self.live(key).map(|node| node.output)
    }

    /// The screenshots the shell asked to open since the last call, by the session's numbers.
    pub fn take_screenshot_opens(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.screenshots.opens)
    }

    /// The session saved a screenshot of `output` (`width` × `height` pixels) at `path`: the shell
    /// of that output is told, to show its thumbnail.
    pub fn screenshot_saved(
        &mut self,
        screenshot: u32,
        output: OutputId,
        (width, height): (u32, u32),
        path: &str,
    ) {
        if path.is_empty() || path.len() > MAX_SCREENSHOT_PATH_BYTES {
            return;
        }
        let Some(shell) = self.shortcut_shell(output) else {
            return;
        };
        let chunks = TextChunk::split(path);
        let count = chunks.len();
        for (index, chunk) in chunks.iter().enumerate() {
            let Ok(index) = u8::try_from(index) else {
                return;
            };
            self.emit(
                shell,
                Event::ScreenshotSaved {
                    screenshot,
                    output,
                    width,
                    height,
                    index,
                    last: usize::from(index) + 1 == count,
                    text: PreeditText::new(chunk.as_str()).expect("a chunk fits"),
                },
            );
        }
    }
}
