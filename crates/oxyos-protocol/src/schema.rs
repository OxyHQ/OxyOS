//! The authoritative message schema. `docs/display-protocol.mdx` documents the same table; the
//! `schema_table_matches_documentation` test keeps them in step.

use crate::wire::{
    AccessibleAction, AccessibleActions, AccessibleField, AccessibleRole, AccessibleState,
    AuthOutcome, BatteryState, BluetoothState, Clock, Control, DismissReason, ErrorCode, Format,
    GesturePhase, MenuField, MenuFlags, MenuKind, NotificationField, NotificationOutcome, Outcome,
    PanelEdge, PointerKind, PointerShape, Politeness, Positioner, PreeditStyle, PreeditText,
    ReaderCommand, Rect, SecretChunk, ShellKind, Size, SystemShortcut, TextChunk, TextHints,
    TextPurpose, TouchKind, WifiState, Wire,
};
use crate::{invalid, MAX_PACKET_BYTES};
use std::io;

macro_rules! messages {
    ($(#[$meta:meta])* $name:ident { $($opcode:literal => $variant:ident { $($field:ident: $type:ty),* $(,)? }),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name {
            $($variant { $($field: $type),* }),*
        }

        impl $name {
            /// (opcode, name, payload bytes including the opcode) for every message.
            pub const SCHEMA: &'static [(u8, &'static str, usize)] = &[
                $(($opcode, stringify!($variant), 1 $(+ <$type as Wire>::SIZE)*)),*
            ];

            pub fn opcode(&self) -> u8 {
                match self {
                    $(Self::$variant { .. } => $opcode),*
                }
            }

            pub(crate) fn encode(&self) -> io::Result<Vec<u8>> {
                let mut out = Vec::with_capacity(MAX_PACKET_BYTES);
                match self {
                    $(Self::$variant { $($field),* } => {
                        out.push($opcode);
                        $(Wire::put($field, &mut out);)*
                    })*
                }
                // Encoding goes through the same validation as decoding, so a peer never emits a
                // message it would itself reject.
                Self::decode(&out)?;
                Ok(out)
            }

            pub(crate) fn decode(body: &[u8]) -> io::Result<Self> {
                let opcode = *body.first().ok_or_else(|| invalid("empty packet"))?;
                match opcode {
                    $($opcode => {
                        let expected = 1 $(+ <$type as Wire>::SIZE)*;
                        if body.len() != expected {
                            return Err(invalid("invalid message length"));
                        }
                        #[allow(unused_mut)]
                        let mut offset = 1;
                        $(
                            let $field = <$type as Wire>::get(&body[offset..offset + <$type as Wire>::SIZE])?;
                            offset += <$type as Wire>::SIZE;
                        )*
                        debug_assert_eq!(offset, expected);
                        Ok(Self::$variant { $($field),* })
                    })*
                    _ => Err(invalid("unknown opcode")),
                }
            }
        }
    };
}

messages! {
    /// Client → compositor. Surface identifiers are chosen by the client and are connection-local.
    Request {
        1 => Hello { min_version: u16, max_version: u16 },
        2 => CreateToplevel { surface: u32, size: Size },
        3 => CreatePopup { surface: u32, parent: u32, positioner: Positioner },
        4 => CreateDialog { surface: u32, parent: u32, size: Size },
        5 => CreateShellSurface { surface: u32, kind: ShellKind, edge: PanelEdge, output: u32, size: Size, exclusive: u32 },
        6 => CreateLockSurface { surface: u32, output: u32 },
        7 => GrabPopup { surface: u32, serial: u64 },
        8 => Reposition { surface: u32, positioner: Positioner },
        9 => SetInputRegion { surface: u32, region: Rect },
        10 => Submit { surface: u32, sequence: u64, buffer: u64, configure: u64, extent: Size },
        11 => Destroy { surface: u32 },
        12 => BeginMove { surface: u32, serial: u64 },
        13 => SetFullscreen { surface: u32, enabled: bool },
        14 => Goodbye {},
        15 => AttachBuffer { buffer: u64, format: Format, extent: Size, stride: u32, offset: u32 },
        16 => DestroyBuffer { buffer: u64 },
        // Accessibility (ADR 0003): one snapshot per surface, replaced at commit.
        17 => AccessibilityBegin { surface: u32, nodes: u32 },
        18 => AccessibleNode { surface: u32, index: u32, parent: u32, role: AccessibleRole, state: AccessibleState, actions: AccessibleActions, rect: Rect },
        19 => AccessibleText { surface: u32, index: u32, field: AccessibleField, text: TextChunk },
        20 => AccessibilityCommit { surface: u32 },
        // Assistive technology only (the `accessibility` grant): a node of another client.
        // `custom`: a Custom action's index among the node's; `start` and `end`: SetSelection's
        // range, in bytes of the node's value; 0 for any other action.
        21 => AccessibilityAction { client: u32, surface: u32, index: u32, action: AccessibleAction, custom: u32, start: u32, end: u32 },
        // What changed in the next Submit's buffer since the surface's previous one, in buffer
        // pixels (ADR 0005); none damages the whole surface.
        22 => Damage { surface: u32, rect: Rect },
        // The clipboard (ADR 0007). An offer of content the client holds, begun with a user
        // input's serial, its media types, then committed as the clipboard while the client has
        // keyboard focus; or the owner empties it.
        23 => ClipboardOffer { offer: u32, serial: u64 },
        24 => ClipboardType { offer: u32, mime: TextChunk },
        25 => ClipboardCommit { offer: u32 },
        26 => ClipboardClear { serial: u64 },
        // A paste: one type of the clipboard's content, written by its owner into the pipe this
        // request carries, while the client has keyboard focus, with a user input's serial.
        27 => ClipboardReceive { selection: u32, mime: TextChunk, serial: u64 },
        // Text input (ADR 0008). A focused field describes itself, double-buffered: its purpose
        // and hints, the text around the caret (chunks, appended, at most 4,000 bytes), the
        // selection within it in bytes, the caret's rectangle; TextInputCommit applies them, its
        // serial counting the client's commits upwards.
        28 => TextInputEnable { surface: u32 },
        29 => TextInputDisable { surface: u32 },
        30 => TextInputPurpose { surface: u32, purpose: TextPurpose, hints: TextHints },
        31 => TextInputSurrounding { surface: u32, text: TextChunk },
        32 => TextInputSelection { surface: u32, anchor: u32, cursor: u32 },
        33 => TextInputCaret { surface: u32, rect: Rect },
        34 => TextInputCommit { surface: u32, serial: u32 },
        // The input method (a client granted `input_method`, ADR 0008): its answer for each key
        // it was given (`used`: it took the key; otherwise the key goes on to the application),
        // and a change for the field, in parts applied at InputMethodCommit, whose serial is the
        // field's commit (InputMethodDone) it was made for.
        35 => InputMethodKeyHandled { serial: u64, used: bool },
        36 => InputMethodDelete { before: u32, after: u32 },
        37 => InputMethodCommitText { text: TextChunk },
        38 => InputMethodPreeditText { text: TextChunk },
        39 => InputMethodPreeditCursor { begin: u32, end: u32 },
        40 => InputMethodPreeditSegment { begin: u32, end: u32, style: PreeditStyle },
        41 => InputMethodCommit { serial: u32 },
        // The input method's candidates: one surface of this size the compositor places next to
        // the caret of the field it serves, shown only while it serves one. Its configure carries
        // the size alone: the input method never learns where the caret is.
        42 => CreateInputCandidates { surface: u32, size: Size },
        // A window's title and application id (ADR 0009), each in chunks of up to 32 bytes:
        // `index` 0 begins it, `last` applies it. The application id is the application's own
        // claim until launch records give it; the shell shows it, nothing is authorized by it.
        43 => SetTitle { surface: u32, index: u8, last: bool, text: PreeditText },
        44 => SetAppId { surface: u32, index: u8, last: bool, text: PreeditText },
        // The session's shell (the `shell` grant): acting on windows by the numbers the
        // compositor gave them, and on workspaces (numbered from 0).
        45 => FocusWindow { window: u32 },
        46 => CloseWindow { window: u32 },
        47 => MinimizeWindow { window: u32 },
        48 => MoveWindowToWorkspace { window: u32, workspace: u32 },
        49 => ActivateWorkspace { workspace: u32 },
        50 => AddWorkspace {},
        51 => RemoveWorkspace { workspace: u32 },
        // The pointer's shape while it is over the surface (the arrow until one is asked for).
        52 => SetPointerShape { surface: u32, shape: PointerShape },
        // The lock (ADR 0010). The shell (its `shell` grant) locks the session; only the session
        // unlocks it. A lock surface, while locked, gives a password in chunks (at most
        // MAX_SECRET_BYTES, appended) and Authenticate submits it for the session to check;
        // AuthenticationResult answers. The password is the compositor's and the session's
        // alone: nothing else ever sees it.
        53 => LockSession {},
        54 => AuthenticateSecret { surface: u32, secret: SecretChunk },
        55 => Authenticate { surface: u32 },
        // Accessibility (ADR 0003): a text field's selection in a snapshot being sent (bytes of
        // its value; never a secure field's), and an announcement in chunks of up to 32 bytes
        // (`index` 0 begins it, `last` sends it) for assistive technologies.
        56 => AccessibleSelection { surface: u32, index: u32, start: u32, end: u32 },
        57 => AccessibilityAnnounce { surface: u32, index: u8, last: bool, politeness: Politeness, text: PreeditText },
        // The session's shell (its `shell` grant) asks the session to start an application of
        // its registry by id (ADR 0013), in chunks as SetAppId's; `serial`, the shell's own,
        // names the request in LaunchResult. The session starts only what its registry lists.
        58 => LaunchApplication { serial: u32, index: u8, last: bool, text: PreeditText },
        // A toplevel's own window (ADR 0016): the logical sizes it can take (a zero is no bound;
        // the compositor never configures it outside them), zoomed to fill the work area or
        // restored, and minimized as the shell minimizes it.
        59 => SetSizeLimits { surface: u32, min_width: u32, min_height: u32, max_width: u32, max_height: u32 },
        60 => SetMaximized { surface: u32, enabled: bool },
        61 => Minimize { surface: u32 },
        // Settings (the `settings` grant, ADR 0019) changes one of the user's preferences:
        // `key=value` in chunks as SetAppId's; `serial`, its own, names the change in
        // PreferenceResult. The session validates it, keeps it and applies it.
        62 => SetPreference { serial: u32, index: u8, last: bool, text: PreeditText },
        // A toplevel's menus (ADR 0021), sent as an accessibility snapshot is: MenusBegin with the
        // commit's `serial` (above every earlier one of the window) and how many entries follow,
        // each entry in order (a menu of the bar has no parent; any other's parent is an earlier
        // menu or submenu), their labels and shortcuts in chunks, then MenusCommit, which
        // replaces the window's menus; none (0 entries) is none. A command is chosen by its
        // index in the commit that declared it, so a choice made before a recommit is refused.
        80 => MenusBegin { surface: u32, serial: u32, items: u32 },
        81 => MenuItem { surface: u32, index: u32, parent: u32, kind: MenuKind, flags: MenuFlags },
        82 => MenuText { surface: u32, index: u32, field: MenuField, text: TextChunk },
        83 => MenusCommit { surface: u32 },
        // The session's shell (its `shell` grant) chooses a command of the active window's menus,
        // by the window's number, the commit's serial and the command's index in it, as it was
        // told them (MenuBar). Anything else is answered with nothing.
        84 => ActivateMenuItem { window: u32, serial: u32, index: u32 },
        // The session's shell (its `shell` grant, ADR 0022): the windows one of its shell surfaces
        // shows live, each by the number the compositor gave it, which the compositor draws over
        // the surface's own pixels: within `rect` (surface-local logical pixels), fitted to it
        // with its proportions kept and centred, cut to `clip`, its corners rounded by `radius`,
        // at `opacity`. ClearWindowViews begins the surface's list anew, each PlaceWindowView
        // adds one above those before it (at most MAX_WINDOW_VIEWS), and the list is shown with
        // the surface's next Submit. The shell never receives a window's pixels.
        90 => ClearWindowViews { surface: u32 },
        91 => PlaceWindowView { surface: u32, window: u32, rect: Rect, clip: Rect, radius: u32, opacity: u8 },
        // A login screen (ADR 0020): the account a lock surface's next password is for, by its
        // Unix user's name (at most 31 bytes), given before the password's chunks while locked;
        // without it the password is the session's own user's. The authentication path decides
        // what it may check; a session checks only its own user's.
        100 => AuthenticateAccount { surface: u32, user: PreeditText },
        // The session's shell (its `shell` grant, ADR 0024): hides the application of a window (all
        // its windows leave the screen, not into the dock, until the application is activated:
        // FocusWindow of any of them shows them again), or quits it (each of its windows is asked
        // to close, CloseRequested, and may refuse). An application is the windows the shell sees
        // as one: the same application id, launched or claimed alike.
        110 => HideApplication { window: u32 },
        111 => QuitApplication { window: u32 },
        // The session's shell gives the keyboard focus to one of its own panels or overlays (the
        // launcher's search), citing the serial of an input it received: a press, a key or a
        // SystemShortcut. One it did not receive lately is ignored.
        112 => FocusShellSurface { surface: u32, serial: u64 },
        // The session's shell (its `shell` grant, ADR 0025), from its Control Center: a level (0
        // to FULL_LEVEL, all of it) or a volume's mute, Wi-Fi on or off, the appearance light or
        // dark (a change of the user's `appearance` preference, answered as SetPreference's with a
        // PreferenceResult naming `serial`), and the system's state looked at again now (the
        // Control Center opened). The session validates and applies each and tells what came of
        // it (ControlLevel and the rest); while the session is locked none is taken.
        120 => SetControlLevel { control: Control, level_65536: u32 },
        121 => SetControlMuted { control: Control, muted: bool },
        122 => SetWifiEnabled { enabled: bool },
        123 => SetAppearance { serial: u32, dark: bool },
        124 => RefreshControls {},
        // Notifications (ADR 0026). An application posts one as its `serial`, its own: its texts in
        // chunks (`index` 0 begins one, `last` ends it; `action` numbers an action's from 1, 0 for
        // the others), then PostNotification, which NotificationResult answers. `banner`: shown as
        // it arrives, as the session's preferences allow (else only kept in the Notification
        // Center); `background`: bit n-1 set, action n answers without bringing the application
        // forward. Only a process the session started (its launch record) is an application.
        130 => NotificationText { serial: u32, field: NotificationField, action: u8, index: u8, last: bool, text: PreeditText },
        131 => PostNotification { serial: u32, banner: bool, background: u8 },
        // An application withdraws one of its own notifications by the number the session gave it
        // (0: every one of its own); the session's shell (its `shell` grant) removes any (0: all).
        132 => WithdrawNotification { notification: u32 },
        // The session's shell: the reply typed for a notification, in chunks, before the activation
        // it goes with; and the person pressing it (`action` 0) or one of its actions, citing the
        // input's serial, a press or a key the shell received.
        133 => NotificationReply { notification: u32, index: u8, last: bool, text: PreeditText },
        134 => ActivateNotification { notification: u32, action: u8, serial: u64 },
        // The session's shell (its `shell` grant), from its Control Center: Do Not Disturb on or off,
        // a change of the user's `do_not_disturb` preference answered as SetPreference's with a
        // PreferenceResult naming `serial`, as SetAppearance is. While locked it is refused.
        135 => SetDoNotDisturb { serial: u32, enabled: bool },
        // Settings (the `settings` grant, ADR 0018): a display's arrangement, mode and scale, each
        // answered as SetPreference's with a PreferenceResult naming `serial`. The display is named
        // by its output, as Display told it. A mode is one of the display's modes (DisplayMode),
        // shown at once and kept only once ConfirmDisplays keeps it: without that, within the
        // seconds DisplayConfirmation said, the display goes back to what it showed before. A scale
        // is one of the display's choices (DisplayScale). An origin is where the display is wanted
        // in the global space (logical pixels); the session places it beside the others, touching
        // one, never over any. The primary display has the dock and the windows of a display that
        // goes.
        140 => SetDisplayMode { serial: u32, output: u32, width: u32, height: u32, refresh_mhz: u32 },
        141 => SetDisplayScale { serial: u32, output: u32, scale_120: u32 },
        142 => SetDisplayOrigin { serial: u32, output: u32, x: i32, y: i32 },
        143 => SetPrimaryDisplay { serial: u32, output: u32 },
        144 => ConfirmDisplays { serial: u32, keep: bool },
        // Key text (ADR 0029): the connection takes the text each key types with the key itself,
        // in KeyText events, for a client whose key events must carry their meaning (a browser's
        // keydown has its `key`). That text is then never sent again as Text or in a text input
        // transaction; text no key typed (an input method's) still comes as before.
        150 => SetKeyText { enabled: bool },
        // Installing applications (ADR 0030). A client with the `store`, `updates` or `privacy`
        // grant asks the session: one request of the session's vocabulary as JSON text in chunks
        // (`index` 0 begins it, `last` completes it), citing `input`, the serial of a user input
        // it received (0 for none): the session refuses what changes anything without one. The
        // answers come as SystemText naming `serial`; without one of those grants, the answer is
        // a refusal.
        160 => SystemRequest { serial: u32, input: u64, index: u16, last: bool, text: PreeditText },
        // The session's shell (its `shell` grant): the person's answer to a system sheet
        // (SheetText), citing `serial`, a press or a key the shell received; an acceptance
        // without one is ignored.
        161 => SheetAnswer { sheet: u32, accepted: bool, serial: u64 },
        // An application asks to use a capability asked at first use (ADR 0030, decision 8:
        // `capability` 1 notifications, 2 files, 3 camera, 4 microphone, 5 location), citing a
        // user input; answered by CapabilityResult naming `serial`.
        162 => RequestCapability { serial: u32, input: u64, capability: u8 },
    }
}

messages! {
    /// Compositor → client. Popup configures carry a position relative to the parent; clients
    /// never learn global coordinates or other clients' geometry. An input event's `time_ns` is
    /// when its device registered it, on CLOCK_MONOTONIC (libinput's time: the kernel's evdev
    /// timestamp), as Wayland's input events carry `time` and Android's MotionEvent `eventTime`:
    /// the client times input by it, never by when it got round to reading it.
    Event {
        1 => Welcome { version: u16 },
        2 => Configure { surface: u32, serial: u64, rect: Rect, scale_120: u32, fullscreen: bool, maximized: bool },
        3 => PopupDone { surface: u32, reason: DismissReason },
        4 => Pointer { surface: u32, serial: u64, kind: PointerKind, x: i32, y: i32, button: u32, time_ns: u64 },
        5 => Key { surface: u32, serial: u64, key: u32, pressed: bool, time_ns: u64 },
        6 => Focus { surface: u32, focused: bool },
        7 => Touch { surface: u32, serial: u64, kind: TouchKind, id: u32, x: i32, y: i32, time_ns: u64 },
        8 => Feedback { surface: u32, sequence: u64, outcome: Outcome, timestamp_ns: u64, clock: Clock },
        9 => Released { surface: u32, buffer: u64 },
        10 => ProtocolError { code: ErrorCode },
        11 => Scroll { surface: u32, serial: u64, dx: i32, dy: i32, time_ns: u64 },
        12 => Text { surface: u32, serial: u64, text: TextChunk },
        13 => Preedit { surface: u32, serial: u64, text: PreeditText, cursor: u8 },
        // To the application: an assistive technology's action on one of its nodes.
        // `serial` is an activation, like a key press: a popup opened by it may grab with it.
        14 => AccessibilityAction { surface: u32, serial: u64, index: u32, action: AccessibleAction, custom: u32, start: u32, end: u32 },
        // To an assistive technology only: committed snapshots, global rectangles.
        15 => AccessibilityTree { client: u32, surface: u32, parent: u32, nodes: u32 },
        16 => AccessibleNode { client: u32, surface: u32, index: u32, parent: u32, role: AccessibleRole, state: AccessibleState, actions: AccessibleActions, rect: Rect },
        17 => AccessibleText { client: u32, surface: u32, index: u32, field: AccessibleField, text: TextChunk },
        18 => AccessibilityCommitted { client: u32, surface: u32 },
        19 => AccessibilityRemoved { client: u32, surface: u32 },
        // To the client with keyboard focus: what the clipboard holds now (`selection` names it,
        // not its owner), its types one by one, or that it is empty.
        20 => ClipboardTypes { selection: u32, count: u8 },
        21 => ClipboardType { selection: u32, mime: TextChunk },
        22 => ClipboardEmpty {},
        // To the clipboard's owner: write that type of the offer into the pipe this event
        // carries, then close it.
        23 => ClipboardSend { offer: u32, mime: TextChunk },
        // To a focused field with text input enabled (ADR 0008), one change applied whole at
        // TextInputDone, in this order: delete bytes around the selection, insert the committed
        // text (chunks) at it, then replace the composition with the new one (chunks; none
        // ends it), its cursor range and its styled segments, in bytes. Done's `serial` is the
        // input's, as a key press's; `commit` the field's commit the change was made for.
        24 => TextInputDelete { surface: u32, before: u32, after: u32 },
        25 => TextInputCommitText { surface: u32, text: TextChunk },
        26 => TextInputPreeditText { surface: u32, text: TextChunk },
        27 => TextInputPreeditCursor { surface: u32, begin: u32, end: u32 },
        28 => TextInputPreeditSegment { surface: u32, begin: u32, end: u32, style: PreeditStyle },
        29 => TextInputDone { surface: u32, serial: u64, commit: u32 },
        // To the input method: the focused field it may serve, never its surface or client.
        // Activate begins one; each state follows as its purpose, surrounding text (chunks) and
        // selection, closed by InputMethodDone with the field's commit serial; Deactivate ends
        // it. While one is active it receives the keys first (`text` what the layout types, empty
        // if nothing) and answers each with InputMethodKeyHandled.
        30 => InputMethodActivate {},
        31 => InputMethodDeactivate {},
        32 => InputMethodPurpose { purpose: TextPurpose, hints: TextHints },
        33 => InputMethodSurrounding { text: TextChunk },
        34 => InputMethodSelection { anchor: u32, cursor: u32 },
        35 => InputMethodDone { serial: u32 },
        36 => InputMethodKey { serial: u64, key: u32, pressed: bool, text: PreeditText, time_ns: u64 },
        // To the session's shell (ADR 0009): each window, by a number the compositor gives it
        // (never a surface id), as it appears and changes, its title and application id in chunks
        // as SetTitle's, its closing; and the workspaces and the active one.
        // `launched`: the session started the window's process, so its application id is the
        // registry's (ADR 0013); otherwise it is the application's own claim.
        37 => WindowState { window: u32, workspace: u32, minimized: bool, focused: bool, launched: bool },
        38 => WindowTitle { window: u32, index: u8, last: bool, text: PreeditText },
        39 => WindowAppId { window: u32, index: u8, last: bool, text: PreeditText },
        40 => WindowClosed { window: u32 },
        41 => Workspaces { count: u32, active: u32 },
        // To an application: the shell asks it to close the window; it may refuse.
        42 => CloseRequested { surface: u32 },
        // A touchpad pinch, to the surface under the pointer when it began, for as long as it
        // lasts: at (x, y), the pointer (surface-local, as a Pointer event's), which a pinch does
        // not move; `scale_65536` the fingers' spread over their spread when it began (65536 = as
        // they started); `rotation_256` how far they turned since, in 1/256 degrees, clockwise.
        43 => Pinch { surface: u32, serial: u64, phase: GesturePhase, x: i32, y: i32, scale_65536: u32, rotation_256: i32, time_ns: u64 },
        // To clients with the `lock` grant (ADR 0010): the session locked, when it does and when
        // they connect while it is, so they make their lock surfaces; and unlocked.
        44 => LockState { locked: bool },
        // To the lock surface that submitted a password: what came of it, and the seconds until
        // another may be tried (0: at once; a fifth rejection in a row begins the waits).
        45 => AuthenticationResult { surface: u32, outcome: AuthOutcome, wait_s: u32 },
        // To the client with keyboard focus: whether Caps Lock is on, whenever it changes, and
        // after its Focus when it is on (a surface gains focus with it off unless told; a
        // password field says it is on, ADR 0010).
        46 => CapsLock { surface: u32, on: bool },
        // To an assistive technology only (ADR 0003): a text field's selection in the snapshot
        // being sent, and an application's announcement, chunk by chunk as it was sent.
        47 => AccessibleSelection { client: u32, surface: u32, index: u32, start: u32, end: u32 },
        48 => AccessibilityAnnouncement { client: u32, surface: u32, index: u8, last: bool, politeness: Politeness, text: PreeditText },
        // To the shell: whether the application its LaunchApplication `serial` named was started.
        49 => LaunchResult { serial: u32, launched: bool },
        // To the screen reader (its `screen_reader` grant, ADR 0014): a command of its keys, which
        // the compositor took from the keyboard (Ctrl+Alt with an arrow or Space).
        50 => ScreenReaderCommand { command: ReaderCommand },
        // To a toplevel (ADR 0016): it became, or stopped being, the active window, the one that
        // has the keyboard focus itself or through its popups or dialogs. The shell's panels
        // taking focus leave it active; its overlays and the lock do not.
        51 => Activated { surface: u32, active: bool },
        // To the client whose SetPreference `serial` named the change (ADR 0019): whether the
        // session took it.
        52 => PreferenceResult { serial: u32, accepted: bool },
        // To every client, when it connects and whenever they change (ADR 0019): the preferences
        // that change how it draws. `dark`, the appearance (an automatic one resolved by the
        // session: dark while the sun is down where the system's time zone is); every transition
        // takes no time; glass is drawn opaque; Bloom's accent colour, by name.
        53 => Preferences { dark: bool, reduce_motion: bool, reduce_transparency: bool, accent: PreeditText },
        // To clients with the `settings` grant (ADR 0019): each of the user's preferences as it is
        // kept, `key=value` in chunks as WindowTitle's, all of them when the grant is given and
        // each one that changes.
        54 => Preference { index: u8, last: bool, text: PreeditText },
        // To the session's shell (ADR 0018): each output, as it is added and whenever it changes,
        // and when one connects; its rectangle in the global space (logical pixels), its scale in
        // 120ths and refresh in mHz, and whether it is the primary (the first of the arrangement,
        // which has the dock); and its removal.
        55 => Output { output: u32, rect: Rect, scale_120: u32, refresh_mhz: u32, primary: bool },
        56 => OutputRemoved { output: u32 },
        // To every surface's client (ADR 0018): the refresh of the output the surface is on, in
        // mHz, before its first Configure and whenever it changes (it moved to another output, or
        // its display is driven at another mode): the display's frames, which its client paces
        // its own by (a frame of animation each refresh), never an assumed 60 a second.
        57 => Refresh { surface: u32, refresh_mhz: u32 },
        // To the session's shell (ADR 0021): the active window's menus, as that window committed
        // them, whenever they or the active window change: MenuBar with the window's number (0:
        // none is active), the commit's serial (0: it declared none) and how many entries follow,
        // the entries in order and their texts in chunks as the window sent them, then
        // MenuBarCommitted. A shell that connects is told them at once.
        70 => MenuBar { window: u32, serial: u32, items: u32 },
        71 => MenuBarItem { window: u32, index: u32, parent: u32, kind: MenuKind, flags: MenuFlags },
        72 => MenuBarText { window: u32, index: u32, field: MenuField, text: TextChunk },
        73 => MenuBarCommitted { window: u32 },
        // To an application: the person chose one of its commands in the shell's bar, the entry
        // `index` of its commit `menus`. `serial` is the choice, a user input as a key press's is.
        74 => MenuItemActivated { surface: u32, serial: u64, menus: u32, index: u32 },
        // To the session's shell (ADR 0022): a window's size in logical pixels, when it is first
        // told and whenever it changes, so the shell shows it in its proportions.
        80 => WindowSize { window: u32, size: Size },
        // To the session's shell of `output` (its `shortcuts` grant, ADR 0024): a system shortcut
        // the compositor took from the keyboard, for the shell to show: the application switcher's
        // and the search's. `serial` is a user input of the shell's, as a key press's is.
        100 => SystemShortcut { shortcut: SystemShortcut, serial: u64, output: u32 },
        // To the session's shell (ADR 0024), after the window's WindowState, when either changes:
        // how recently it was the active window (a rising count of activations; 0: never), and
        // whether it is hidden with its application (minimized, but not into the dock).
        101 => WindowUse { window: u32, used: u32, hidden: bool },
        // To the session's shell (its `shell` grant, ADR 0025), when it connects and whenever they
        // change: a level the session controls as it applied or found it (0 to FULL_LEVEL),
        // whether it is muted (a volume) and whether this machine has it at all. `bezel`: a
        // hardware key acted on it (even at its end) with this shell's output in front, and the
        // shell shows the level there for a moment, as a Mac's bezel.
        110 => ControlLevel { control: Control, level_65536: u32, muted: bool, available: bool, bezel: bool },
        // The battery: its state, its charge in percent, whether the machine is on AC power, and
        // the minutes until it is empty (discharging) or full (charging), 0 when unknown.
        111 => Power { battery: BatteryState, percent: u8, on_ac: bool, minutes: u32 },
        // The network: Wi-Fi's state, its access point's signal in percent and name (its SSID as
        // UTF-8, cut to whole characters within one chunk; empty unless connected or connecting),
        // and whether a wired connection is up.
        112 => Network { wifi: WifiState, signal: u8, wired: bool, ssid: PreeditText },
        // Bluetooth: its adapter's state and how many devices are connected to it.
        113 => Bluetooth { state: BluetoothState, connected: u8 },
        // Notifications (ADR 0026). To the application that posted its `serial`: what came of it,
        // and the number the session gave it (0 unless posted).
        120 => NotificationResult { serial: u32, notification: u32, outcome: NotificationOutcome },
        // A notification's texts in chunks, as the request's: to the session's shell, before its
        // Notification; to an application, before its NotificationResponse.
        121 => NotificationText { notification: u32, field: NotificationField, action: u8, index: u8, last: bool, text: PreeditText },
        // To the session's shell, after its texts, as one is posted and when the shell connects:
        // `banner`, show it now (the preferences allow it, the session is unlocked); `actions`,
        // how many it offers; `age_s`, seconds since it was posted; `withheld`, its texts but its
        // application's are kept from the shell while the session is locked, and told on unlock.
        122 => Notification { notification: u32, banner: bool, actions: u8, age_s: u32, withheld: bool },
        // To the application, after its texts (the notification's own, the chosen action's id and
        // the reply typed): the person pressed it (`action` 0) or its action `action`. `serial` is
        // a user input, as a key press's is. Delivered once, to one of the application's clients;
        // the session starts the application first if none runs.
        123 => NotificationResponse { notification: u32, action: u8, serial: u64 },
        // To the session's shell and the application whose it was: it is no longer kept.
        124 => NotificationRemoved { notification: u32 },
        // To the session's shell, when it connects and whenever they change: whether Do Not
        // Disturb silences banners, and whether the lock screen may show what notifications say.
        125 => NotificationPolicy { do_not_disturb: bool, previews: bool },
        // To clients with the `settings` grant (ADR 0018), when the grant is given and whenever
        // they change: each display the session shows, by its output. Its name in chunks (the
        // monitor's own, from its EDID, else its connector's; `index` 0 begins it), its modes
        // (`index` 0 begins the list; pixels, refresh in mHz, and the one the display prefers) and
        // its scale choices (`index` 0 begins them, from the most space to the largest text; in
        // 120ths, and the one its density recommends), then Display, which applies them: its
        // rectangle in the global space, its scale, its mode (an index of its modes), its physical
        // size in millimetres (0 when unknown), whether it is the primary and whether it is built
        // into the machine. Only what changed is sent again. And its removal.
        130 => DisplayName { output: u32, index: u8, last: bool, text: PreeditText },
        131 => DisplayMode { output: u32, index: u32, width: u32, height: u32, refresh_mhz: u32, preferred: bool },
        132 => DisplayScale { output: u32, index: u32, scale_120: u32, recommended: bool },
        133 => Display { output: u32, rect: Rect, scale_120: u32, mode: u32, width_mm: u32, height_mm: u32, primary: bool, builtin: bool },
        134 => DisplayRemoved { output: u32 },
        // To clients with the `settings` grant: the display's mode was changed and is shown, and
        // goes back unless ConfirmDisplays keeps it within `seconds`; 0, it was kept or went back.
        135 => DisplayConfirmation { output: u32, seconds: u32 },
        // To a connection that asked for key text (SetKeyText): a key, as Key, with what the
        // session's layout typed with it (its compose sequences included; empty for a modifier, an
        // editing key, a control character or a dead key).
        150 => KeyText { surface: u32, serial: u64, key: u32, pressed: bool, text: PreeditText, time_ns: u64 },
        // To a client with the `store`, `updates` or `privacy` grant: the session's answer to its
        // SystemRequest `serial`, JSON text in chunks; `serial` 0 is the account's update state as
        // it changes, unasked.
        160 => SystemText { serial: u32, index: u16, last: bool, text: PreeditText },
        // To the session's shell: a system sheet to show (install, remove, consent, permission),
        // JSON text in chunks; `last` shows it. Only the shell sees it, and only a SheetAnswer
        // citing an input the shell received answers it.
        161 => SheetText { sheet: u32, index: u16, last: bool, text: PreeditText },
        // To the session's shell: the sheet is no longer asked for (its asker went).
        162 => SheetDismissed { sheet: u32 },
        // To the application whose RequestCapability `serial` it answers.
        163 => CapabilityResult { serial: u32, granted: bool },
    }
}

impl Event {
    /// Whether this event carries exactly one file descriptor (a pipe to write a clipboard's
    /// content into); every other event carries none.
    pub fn carries_fd(&self) -> bool {
        matches!(self, Event::ClipboardSend { .. })
    }
}

impl Request {
    /// Whether this request must carry exactly one file descriptor; every other request must
    /// carry none.
    pub fn carries_fd(&self) -> bool {
        matches!(
            self,
            Request::AttachBuffer { .. } | Request::ClipboardReceive { .. }
        )
    }

    /// The validated buffer layout of an AttachBuffer.
    pub fn buffer_layout(&self) -> Option<crate::BufferLayout> {
        match *self {
            Request::AttachBuffer {
                format,
                extent,
                stride,
                offset,
                ..
            } => Some(crate::BufferLayout {
                format,
                extent,
                stride,
                offset,
            }),
            _ => None,
        }
    }
}

const _: () = {
    // Fails to compile if a message outgrows the bounded packet.
    let mut index = 0;
    while index < Request::SCHEMA.len() {
        assert!(Request::SCHEMA[index].2 <= MAX_PACKET_BYTES);
        index += 1;
    }
    let mut index = 0;
    while index < Event::SCHEMA.len() {
        assert!(Event::SCHEMA[index].2 <= MAX_PACKET_BYTES);
        index += 1;
    }
};
