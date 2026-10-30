//! C ABI over `oxyos-client` for native runtimes written in C or C++ (the React Native host that
//! runs Bloom), and over `oxyos-net` for their HTTP ([`net`]), `oxyos-image` for their images
//! ([`images`]), `oxyos-vector` for their vector graphics ([`vector`]), `oxyos-gestures` for
//! react-native-gesture-handler's recognizers ([`gestures`]) and `oxyos-intl` for their `Intl`
//! ([`intl`]). The protocol,
//! transport, buffer handling, network, decoding and rasterizing stay in the safe Rust crates; this layer only converts between C values and theirs. `include/oxyos_client.h`
//! declares it.
//!
//! The workspace forbids `unsafe`; this crate denies it and allows it only on the items below,
//! each with the caller contract it relies on.

use oxyos_client::{
    Accessible, Client, Field, Menu, NotificationActionPost, NotificationPost, Poll,
};
use oxyos_protocol::{
    AccessibleActions, AccessibleRole, AccessibleState, Adjust, Anchor, Control, Event, Format,
    Gravity, MenuFlags, MenuKind, PanelEdge, Politeness, Positioner, PreeditText, Rect, Request,
    SecretChunk, ShellKind, Size, TextChunk, TextHints, TextPurpose, FULL_LEVEL, TEXT_CHUNK_BYTES,
};
use rustix::fd::{AsFd, AsRawFd};
use std::cell::RefCell;
use std::ffi::{c_char, c_int, CStr, CString};
use std::path::Path;

pub mod accounts;
pub mod application;
pub mod apps;
pub mod files;
pub mod gestures;
pub mod images;
pub mod intl;
pub mod net;
pub mod solar;
#[cfg(test)]
mod tests;
pub mod vector;

/// The type text is copied and pasted as.
pub const CLIPBOARD_TEXT: &str = "text/plain;charset=utf-8";

/// Event kinds; the values are the protocol's event opcodes.
pub const OXYOS_EVENT_CONFIGURE: u32 = 2;
pub const OXYOS_EVENT_POPUP_DONE: u32 = 3;
pub const OXYOS_EVENT_POINTER: u32 = 4;
pub const OXYOS_EVENT_KEY: u32 = 5;
pub const OXYOS_EVENT_FOCUS: u32 = 6;
pub const OXYOS_EVENT_TOUCH: u32 = 7;
pub const OXYOS_EVENT_FEEDBACK: u32 = 8;
pub const OXYOS_EVENT_RELEASED: u32 = 9;
pub const OXYOS_EVENT_SCROLL: u32 = 11;
pub const OXYOS_EVENT_TEXT: u32 = 12;
pub const OXYOS_EVENT_PREEDIT: u32 = 13;
pub const OXYOS_EVENT_ACCESSIBILITY_ACTION: u32 = 14;
pub const OXYOS_EVENT_CLIPBOARD_TYPES: u32 = 20;
pub const OXYOS_EVENT_CLIPBOARD_TYPE: u32 = 21;
pub const OXYOS_EVENT_CLIPBOARD_EMPTY: u32 = 22;
/// A transaction for a field with text input enabled is complete (its Done's opcode); its
/// contents come from `oxyos_client_text_transaction`.
pub const OXYOS_EVENT_TEXT_INPUT: u32 = 29;
/// To the session's shell (ADR 0009): a window's state, a chunk of its title or application id,
/// its closing, and the workspaces.
pub const OXYOS_EVENT_WINDOW_STATE: u32 = 37;
pub const OXYOS_EVENT_WINDOW_TITLE: u32 = 38;
pub const OXYOS_EVENT_WINDOW_APP_ID: u32 = 39;
pub const OXYOS_EVENT_WINDOW_CLOSED: u32 = 40;
pub const OXYOS_EVENT_WORKSPACES: u32 = 41;
/// To the session's shell (ADR 0022): a window's size in logical pixels (code: the window;
/// width, height), when it is first told and whenever it changes.
pub const OXYOS_EVENT_WINDOW_SIZE: u32 = 80;
/// To an application: the shell asks it to close a window.
pub const OXYOS_EVENT_CLOSE_REQUESTED: u32 = 42;
/// A touchpad pinch over one of the client's surfaces.
pub const OXYOS_EVENT_PINCH: u32 = 43;
/// A pinch's phase (OxyOSEvent.detail of a PINCH event).
pub const OXYOS_PINCH_BEGIN: u32 = 1;
pub const OXYOS_PINCH_UPDATE: u32 = 2;
pub const OXYOS_PINCH_END: u32 = 3;
pub const OXYOS_PINCH_CANCEL: u32 = 4;
/// To a client with the `lock` grant (ADR 0010): whether the session is locked (flag).
pub const OXYOS_EVENT_LOCK_STATE: u32 = 44;
/// To a lock surface: what came of its password (detail, OXYOS_AUTH_*; code: the seconds until
/// another may be tried, 0 at once).
pub const OXYOS_EVENT_AUTHENTICATION_RESULT: u32 = 45;
/// To the surface with keyboard focus: whether Caps Lock is on (flag), when it changes and after
/// its focus when it is on; a surface gains focus with it off unless told.
pub const OXYOS_EVENT_CAPS_LOCK: u32 = 46;
/// To the session's shell: whether the application it asked for was started (ADR 0013): code,
/// its request's serial; flag, started.
pub const OXYOS_EVENT_LAUNCH_RESULT: u32 = 49;
/// A toplevel became, or stopped being, the active window (ADR 0016); `flag` says which.
pub const OXYOS_EVENT_ACTIVATED: u32 = 51;
/// To Settings (ADR 0019): whether the change it asked for was taken: code, its serial; flag,
/// taken.
pub const OXYOS_EVENT_PREFERENCE_RESULT: u32 = 52;
/// To every client (ADR 0019), when it connects and whenever they change: the preferences it
/// draws with. flag: OXYOS_PREFERENCES_* bits; text: Bloom's accent colour, by name.
pub const OXYOS_EVENT_PREFERENCES: u32 = 53;
/// To Settings (ADR 0019): a chunk of one preference as kept, `key=value`: detail, the chunk's
/// index (0 begins it); flag, whether it is the last; text, the chunk.
pub const OXYOS_EVENT_PREFERENCE: u32 = 54;
/// A PREFERENCES event's `flag` bits: dark (an automatic appearance resolved), every transition
/// takes no time, glass is drawn opaque.
pub const OXYOS_PREFERENCES_DARK: u8 = 1;
pub const OXYOS_PREFERENCES_REDUCE_MOTION: u8 = 2;
pub const OXYOS_PREFERENCES_REDUCE_TRANSPARENCY: u8 = 4;
/// To the session's shell (ADR 0021): the active window's menus, begun, entry by entry, text by
/// text, then committed.
pub const OXYOS_EVENT_MENU_BAR: u32 = 70;
pub const OXYOS_EVENT_MENU_BAR_ITEM: u32 = 71;
pub const OXYOS_EVENT_MENU_BAR_TEXT: u32 = 72;
pub const OXYOS_EVENT_MENU_BAR_COMMITTED: u32 = 73;
/// To an application: a command of its menus chosen in the shell's bar.
pub const OXYOS_EVENT_MENU_ITEM_ACTIVATED: u32 = 74;
/// What a menu entry is (`OxyOSMenuItem.kind`, a MENU_BAR_ITEM's `height`).
pub const OXYOS_MENU_MENU: u8 = 1;
pub const OXYOS_MENU_ITEM: u8 = 2;
pub const OXYOS_MENU_SUBMENU: u8 = 3;
pub const OXYOS_MENU_SEPARATOR: u8 = 4;
/// A menu entry's state bits (`OxyOSMenuItem.flags`, a MENU_BAR_ITEM's `flag`).
pub const OXYOS_MENU_DISABLED: u8 = 1;
pub const OXYOS_MENU_CHECKABLE: u8 = 2;
pub const OXYOS_MENU_CHECKED: u8 = 4;
/// Which text of an entry a MENU_BAR_TEXT carries (its `height`).
pub const OXYOS_MENU_LABEL: u32 = 1;
pub const OXYOS_MENU_SHORTCUT: u32 = 2;
/// `parent` of a menu of the bar.
pub const OXYOS_NO_MENU_PARENT: u32 = u32::MAX;
/// A CONFIGURE's `flag` bits: the window fills its output, or is zoomed to its work area.
pub const OXYOS_CONFIGURE_FULLSCREEN: u8 = 1;
pub const OXYOS_CONFIGURE_MAXIMIZED: u8 = 2;
/// To the session's shell (ADR 0018): an output, added or changed (code: its id; x, y, width,
/// height: its rectangle in the global space, logical; scale_120; detail: its refresh in mHz;
/// flag: whether it is the primary, which has the dock), and an output removed (code).
pub const OXYOS_EVENT_OUTPUT: u32 = 55;
pub const OXYOS_EVENT_OUTPUT_REMOVED: u32 = 56;
/// To every surface's client (ADR 0018): the refresh of the output the surface is on (detail, in
/// mHz), before its first CONFIGURE and whenever it changes; its frames are paced by it.
pub const OXYOS_EVENT_REFRESH: u32 = 57;
pub const OXYOS_AUTH_ACCEPTED: u32 = 1;
pub const OXYOS_AUTH_REJECTED: u32 = 2;
pub const OXYOS_AUTH_BUSY: u32 = 3;
pub const OXYOS_AUTH_THROTTLED: u32 = 4;
/// A window's state bits (OxyOSEvent.flag of a WINDOW_STATE event).
pub const OXYOS_WINDOW_MINIMIZED: u8 = 1;
pub const OXYOS_WINDOW_FOCUSED: u8 = 2;
/// The session started the window's process: its application id is the registry's (ADR 0013).
pub const OXYOS_WINDOW_LAUNCHED: u8 = 4;
/// Pointer shapes (`oxyos_client_set_pointer_shape`), the protocol's.
pub const OXYOS_POINTER_ARROW: u8 = 1;
pub const OXYOS_POINTER_TEXT: u8 = 2;
pub const OXYOS_POINTER_HAND: u8 = 3;
pub const OXYOS_POINTER_NOT_ALLOWED: u8 = 4;
pub const OXYOS_POINTER_WAIT: u8 = 5;
pub const OXYOS_POINTER_HELP: u8 = 6;
pub const OXYOS_POINTER_CROSSHAIR: u8 = 7;
pub const OXYOS_POINTER_MOVE: u8 = 8;
pub const OXYOS_POINTER_GRAB: u8 = 9;
pub const OXYOS_POINTER_GRABBING: u8 = 10;
pub const OXYOS_POINTER_RESIZE_HORIZONTAL: u8 = 11;
pub const OXYOS_POINTER_RESIZE_VERTICAL: u8 = 12;
pub const OXYOS_POINTER_RESIZE_TOP_LEFT: u8 = 13;
pub const OXYOS_POINTER_RESIZE_TOP_RIGHT: u8 = 14;
pub const OXYOS_POINTER_ZOOM_IN: u8 = 15;
pub const OXYOS_POINTER_ZOOM_OUT: u8 = 16;
/// What the shell asks of windows and workspaces (`oxyos_client_manage`).
pub const OXYOS_MANAGE_FOCUS: u8 = 1;
pub const OXYOS_MANAGE_CLOSE: u8 = 2;
pub const OXYOS_MANAGE_MINIMIZE: u8 = 3;
pub const OXYOS_MANAGE_MOVE: u8 = 4;
pub const OXYOS_MANAGE_ACTIVATE_WORKSPACE: u8 = 5;
pub const OXYOS_MANAGE_ADD_WORKSPACE: u8 = 6;
pub const OXYOS_MANAGE_REMOVE_WORKSPACE: u8 = 7;
/// Hides or quits the application of the window (ADR 0024).
pub const OXYOS_MANAGE_HIDE_APPLICATION: u8 = 8;
pub const OXYOS_MANAGE_QUIT_APPLICATION: u8 = 9;
/// To the session's shell (ADR 0024): a system shortcut it shows (serial: a user input of its
/// own, which oxyos_client_focus_shell_surface may cite; detail: OXYOS_SHORTCUT_*; code: the
/// output to show it on), and how recently a window was active (code: the window; detail: its
/// latest activation, rising, 0 never; flag: whether it is hidden with its application).
pub const OXYOS_EVENT_SYSTEM_SHORTCUT: u32 = 100;
pub const OXYOS_EVENT_WINDOW_USE: u32 = 101;
/// To the session's shell (ADR 0025): a level the session controls (detail: OXYOS_CONTROL_*;
/// code: 0 to 65536; flag: OXYOS_CONTROL_MUTED, _AVAILABLE, _BEZEL bits), the battery (detail:
/// OXYOS_BATTERY_*; code: percent; width: minutes until empty or full, 0 unknown; flag: on AC
/// power), the network (detail: OXYOS_WIFI_*; code: the access point's signal in percent; flag:
/// a wired connection is up; text: the access point's name) and Bluetooth (detail:
/// OXYOS_BLUETOOTH_*; code: devices connected).
pub const OXYOS_EVENT_CONTROL_LEVEL: u32 = 110;
pub const OXYOS_EVENT_POWER: u32 = 111;
pub const OXYOS_EVENT_NETWORK: u32 = 112;
pub const OXYOS_EVENT_BLUETOOTH: u32 = 113;
/// A control, the protocol's `Control`.
pub const OXYOS_CONTROL_OUTPUT_VOLUME: u8 = 1;
pub const OXYOS_CONTROL_INPUT_VOLUME: u8 = 2;
pub const OXYOS_CONTROL_DISPLAY_BRIGHTNESS: u8 = 3;
pub const OXYOS_CONTROL_KEYBOARD_BRIGHTNESS: u8 = 4;
/// A CONTROL_LEVEL's flag bits.
pub const OXYOS_CONTROL_MUTED: u8 = 1;
pub const OXYOS_CONTROL_AVAILABLE: u8 = 2;
pub const OXYOS_CONTROL_BEZEL: u8 = 4;
/// A POWER's detail, the protocol's `BatteryState`.
pub const OXYOS_BATTERY_ABSENT: u32 = 0;
pub const OXYOS_BATTERY_DISCHARGING: u32 = 1;
pub const OXYOS_BATTERY_CHARGING: u32 = 2;
pub const OXYOS_BATTERY_FULL: u32 = 3;
pub const OXYOS_BATTERY_NOT_CHARGING: u32 = 4;
/// A NETWORK's detail, the protocol's `WifiState`.
pub const OXYOS_WIFI_UNAVAILABLE: u32 = 0;
pub const OXYOS_WIFI_OFF: u32 = 1;
pub const OXYOS_WIFI_DISCONNECTED: u32 = 2;
pub const OXYOS_WIFI_CONNECTING: u32 = 3;
pub const OXYOS_WIFI_CONNECTED: u32 = 4;
/// A BLUETOOTH's detail, the protocol's `BluetoothState`.
pub const OXYOS_BLUETOOTH_UNAVAILABLE: u32 = 0;
pub const OXYOS_BLUETOOTH_OFF: u32 = 1;
pub const OXYOS_BLUETOOTH_ON: u32 = 2;
/// A SYSTEM_SHORTCUT's `detail`, the protocol's `SystemShortcut`.
pub const OXYOS_SHORTCUT_SWITCHER_NEXT: u32 = 1;
pub const OXYOS_SHORTCUT_SWITCHER_PREVIOUS: u32 = 2;
pub const OXYOS_SHORTCUT_SWITCHER_QUIT: u32 = 3;
pub const OXYOS_SHORTCUT_SWITCHER_HIDE: u32 = 4;
pub const OXYOS_SHORTCUT_SWITCHER_DONE: u32 = 5;
pub const OXYOS_SHORTCUT_SWITCHER_CANCEL: u32 = 6;
pub const OXYOS_SHORTCUT_SEARCH: u32 = 7;

/// Notifications (docs/adr/0026-notifications.mdx). To the application that posted one: what came
/// of it (code: the session's number for it, 0 unless posted; detail: OXYOS_NOTIFICATION_POSTED,
/// _REFUSED, _THROTTLED; width: the post's serial).
pub const OXYOS_EVENT_NOTIFICATION_RESULT: u32 = 120;
/// A chunk of a notification's text (code: the notification; height: OXYOS_NOTIFICATION_FIELD_*;
/// width: the action it is of, from 1, else 0; detail: the chunk's index, 0 begins it; flag: the
/// last), to the shell before its NOTIFICATION, to the application before its response.
pub const OXYOS_EVENT_NOTIFICATION_TEXT: u32 = 121;
/// To the session's shell, after its texts (code: the notification; flag: OXYOS_NOTIFICATION_BANNER,
/// _WITHHELD bits; width: how many actions; detail: seconds since it was posted).
pub const OXYOS_EVENT_NOTIFICATION: u32 = 122;
/// To the application, after its texts: the person pressed it (width 0) or its action `width`;
/// serial: a user input of the application's.
pub const OXYOS_EVENT_NOTIFICATION_RESPONSE: u32 = 123;
/// To the shell and the application: the notification `code` is no longer kept.
pub const OXYOS_EVENT_NOTIFICATION_REMOVED: u32 = 124;
/// To the shell (flag: OXYOS_NOTIFICATIONS_DO_NOT_DISTURB, _PREVIEWS bits).
pub const OXYOS_EVENT_NOTIFICATION_POLICY: u32 = 125;
/// A NOTIFICATION_RESULT's `detail`, the protocol's `NotificationOutcome`.
pub const OXYOS_NOTIFICATION_POSTED: u32 = 1;
pub const OXYOS_NOTIFICATION_REFUSED: u32 = 2;
pub const OXYOS_NOTIFICATION_THROTTLED: u32 = 3;
/// A NOTIFICATION_TEXT's `height`, the protocol's `NotificationField`.
pub const OXYOS_NOTIFICATION_FIELD_IDENTIFIER: u32 = 1;
pub const OXYOS_NOTIFICATION_FIELD_TITLE: u32 = 2;
pub const OXYOS_NOTIFICATION_FIELD_SUBTITLE: u32 = 3;
pub const OXYOS_NOTIFICATION_FIELD_BODY: u32 = 4;
pub const OXYOS_NOTIFICATION_FIELD_DATA: u32 = 5;
pub const OXYOS_NOTIFICATION_FIELD_ACTION_ID: u32 = 6;
pub const OXYOS_NOTIFICATION_FIELD_ACTION_LABEL: u32 = 7;
pub const OXYOS_NOTIFICATION_FIELD_ACTION_REPLY: u32 = 8;
pub const OXYOS_NOTIFICATION_FIELD_APPLICATION: u32 = 9;
pub const OXYOS_NOTIFICATION_FIELD_REPLY: u32 = 10;
/// A NOTIFICATION's `flag` bits.
pub const OXYOS_NOTIFICATION_BANNER: u8 = 1;
pub const OXYOS_NOTIFICATION_WITHHELD: u8 = 2;
/// A NOTIFICATION_POLICY's `flag` bits.
pub const OXYOS_NOTIFICATIONS_DO_NOT_DISTURB: u8 = 1;
pub const OXYOS_NOTIFICATIONS_PREVIEWS: u8 = 2;
/// To Settings (its `settings` grant, ADR 0018), each display the session shows, by its output
/// (code), when the grant is given and as it changes: a chunk of its name (detail: the chunk's
/// index, 0 begins it; flag: the last; text), each of its modes (detail: the mode's index, 0 begins
/// the list; width, height: its pixels; x: its refresh in mHz; flag: the display prefers it), each
/// of its scale choices from the most space to the largest text (detail: the choice's index, 0
/// begins them; scale_120; flag: the one its density recommends), then DISPLAY, which applies
/// them (x, y, width, height: its rectangle in the global space, logical; scale_120; detail: its
/// mode, an index of its modes; surface and serial: its physical width and height in mm, 0 when
/// unknown; flag: OXYOS_DISPLAY_PRIMARY, _BUILTIN bits); and its removal.
pub const OXYOS_EVENT_DISPLAY_NAME: u32 = 130;
pub const OXYOS_EVENT_DISPLAY_MODE: u32 = 131;
pub const OXYOS_EVENT_DISPLAY_SCALE: u32 = 132;
pub const OXYOS_EVENT_DISPLAY: u32 = 133;
pub const OXYOS_EVENT_DISPLAY_REMOVED: u32 = 134;
/// To Settings: the display `code`'s mode is on trial, and goes back unless kept
/// (`oxyos_client_confirm_displays`) within `detail` seconds; 0, it was kept or went back.
pub const OXYOS_EVENT_DISPLAY_CONFIRMATION: u32 = 135;
/// A DISPLAY's `flag` bits.
pub const OXYOS_DISPLAY_PRIMARY: u8 = 1;
pub const OXYOS_DISPLAY_BUILTIN: u8 = 2;

/// Pixel formats (DRM fourcc codes), as the protocol's `Format`.
pub const OXYOS_FORMAT_XRGB8888: u32 = 0x3432_5258;
pub const OXYOS_FORMAT_ARGB8888: u32 = 0x3432_5241;

/// Results of `oxyos_client_poll`.
pub const OXYOS_POLL_EVENT: c_int = 1;
pub const OXYOS_POLL_EMPTY: c_int = 0;
pub const OXYOS_POLL_CLOSED: c_int = -1;
pub const OXYOS_ERROR: c_int = -2;

/// One compositor event. Which fields are meaningful depends on `kind`; see `oxyos_client.h`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OxyOSEvent {
    pub kind: u32,
    pub surface: u32,
    pub serial: u64,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_120: u32,
    pub detail: u32,
    pub code: u32,
    pub scale_65536: u32,
    pub rotation_256: i32,
    pub flag: u8,
    /// A text event's UTF-8, NUL-terminated.
    pub text: [u8; TEXT_CHUNK_BYTES + 1],
}

impl Default for OxyOSEvent {
    fn default() -> Self {
        Self {
            kind: 0,
            surface: 0,
            serial: 0,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            scale_120: 0,
            detail: 0,
            code: 0,
            scale_65536: 0,
            rotation_256: 0,
            flag: 0,
            text: [0; TEXT_CHUNK_BYTES + 1],
        }
    }
}

/// A popup's placement, as the protocol's `Positioner`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct OxyOSPositioner {
    pub anchor_x: i32,
    pub anchor_y: i32,
    pub anchor_width: u32,
    pub anchor_height: u32,
    pub width: u32,
    pub height: u32,
    pub anchor: u8,
    pub gravity: u8,
    pub adjust: u8,
    pub offset_x: i32,
    pub offset_y: i32,
}

/// One node of a surface's accessibility snapshot, as `oxyos_client_publish_accessibility` takes it.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OxyOSAccessibleNode {
    pub parent: u32,
    pub role: u8,
    pub state: u16,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub name: *const c_char,
    pub value: *const c_char,
    /// What it offers beyond activate and focus (`AccessibleActions` bits).
    pub actions: u16,
    /// Its custom actions' labels, one per line; null for none.
    pub custom_actions: *const c_char,
    /// A text field's selection, in bytes of its value, when non-zero.
    pub has_selection: u8,
    pub selection_start: u32,
    pub selection_end: u32,
}

/// One of a notification's actions, as `oxyos_client_post_notification` takes it (ADR 0026).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OxyOSNotificationAction {
    /// NUL-terminated UTF-8, not empty.
    pub id: *const c_char,
    pub label: *const c_char,
    /// A reply action's placeholder (NUL-terminated, may be empty); NULL for a button.
    pub reply: *const c_char,
    /// It answers without bringing the application forward.
    pub background: bool,
}

/// A notification an application posts (ADR 0026). Texts are NUL-terminated UTF-8, NULL or empty
/// for none.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OxyOSNotification {
    pub identifier: *const c_char,
    pub title: *const c_char,
    pub subtitle: *const c_char,
    pub body: *const c_char,
    pub data: *const c_char,
    pub actions: *const OxyOSNotificationAction,
    pub action_count: usize,
    /// Shown as a banner as it arrives, as the session allows; else only kept.
    pub banner: bool,
}

/// One entry of a window's menus, as `oxyos_client_set_menus` takes it (ADR 0021).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OxyOSMenuItem {
    /// Index of the menu or submenu it is in, earlier in the list; OXYOS_NO_MENU_PARENT for a
    /// menu of the bar.
    pub parent: u32,
    /// OXYOS_MENU_MENU, _ITEM, _SUBMENU or _SEPARATOR.
    pub kind: u8,
    /// OXYOS_MENU_DISABLED, _CHECKABLE, _CHECKED bits.
    pub flags: u8,
    /// NUL-terminated UTF-8; NULL for none (a separator).
    pub label: *const c_char,
    /// A command's key chord, NUL-terminated; NULL for none.
    pub shortcut: *const c_char,
}

/// A focused text field, as `oxyos_client_text_input` takes it (ADR 0008).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OxyOSTextField {
    pub purpose: u8,
    pub hints: u32,
    pub text: *const c_char,
    pub length: usize,
    pub anchor: u32,
    pub cursor: u32,
    pub has_caret: u8,
    pub caret_x: i32,
    pub caret_y: i32,
    pub caret_width: u32,
    pub caret_height: u32,
}

/// One styled segment of a composition, in bytes of it.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OxyOSPreeditSegment {
    pub begin: u32,
    pub end: u32,
    pub style: u8,
}

/// A transaction for a field, as `oxyos_client_text_transaction` gives it. Its strings and
/// segments stay valid until the next `oxyos_client_poll`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OxyOSTextTransaction {
    pub surface: u32,
    pub delete_before: u32,
    pub delete_after: u32,
    pub cursor_begin: u32,
    pub cursor_end: u32,
    pub commit_serial: u32,
    pub serial: u64,
    pub commit: *const c_char,
    pub commit_length: usize,
    pub preedit: *const c_char,
    pub preedit_length: usize,
    pub segments: *const OxyOSPreeditSegment,
    pub segment_count: usize,
}

/// An opaque connection, owned by the caller between `oxyos_client_connect` and
/// `oxyos_client_close`.
pub struct OxyOSClient {
    client: Client,
    /// The segments of the transaction last completed, as C reads them.
    segments: Vec<OxyOSPreeditSegment>,
}

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

fn fail(message: impl ToString) -> c_int {
    let message = message.to_string().replace('\0', " ");
    LAST_ERROR.with(|last| *last.borrow_mut() = CString::new(message).unwrap_or_default());
    OXYOS_ERROR
}

fn status(result: std::io::Result<()>) -> c_int {
    match result {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

/// Converts a compositor event; `None` for events a client never sees (the handshake).
pub fn convert(event: &Event) -> Option<OxyOSEvent> {
    let mut out = OxyOSEvent::default();
    match *event {
        Event::Configure {
            surface,
            serial,
            rect,
            scale_120,
            fullscreen,
            maximized,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_CONFIGURE,
                surface,
                serial,
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
                scale_120,
                flag: if fullscreen {
                    OXYOS_CONFIGURE_FULLSCREEN
                } else {
                    0
                } | if maximized {
                    OXYOS_CONFIGURE_MAXIMIZED
                } else {
                    0
                },
                ..out
            }
        }
        Event::PopupDone { surface, reason } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_POPUP_DONE,
                surface,
                detail: reason as u32,
                ..out
            }
        }
        Event::Pointer {
            surface,
            serial,
            kind,
            x,
            y,
            button,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_POINTER,
                surface,
                serial,
                x,
                y,
                detail: kind as u32,
                code: button,
                ..out
            }
        }
        Event::Key {
            surface,
            serial,
            key,
            pressed,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_KEY,
                surface,
                serial,
                code: key,
                flag: pressed.into(),
                ..out
            }
        }
        Event::KeyText {
            surface,
            serial,
            key,
            pressed,
            text,
        } => {
            let bytes = text.as_str().as_bytes();
            out.text[..bytes.len()].copy_from_slice(bytes);
            out = OxyOSEvent {
                kind: OXYOS_EVENT_KEY,
                surface,
                serial,
                code: key,
                flag: pressed.into(),
                ..out
            }
        }
        Event::Focus { surface, focused } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_FOCUS,
                surface,
                flag: focused.into(),
                ..out
            }
        }
        Event::Touch {
            surface,
            serial,
            kind,
            id,
            x,
            y,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_TOUCH,
                surface,
                serial,
                x,
                y,
                detail: kind as u32,
                code: id,
                ..out
            }
        }
        Event::Feedback {
            surface,
            sequence,
            outcome,
            ..
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_FEEDBACK,
                surface,
                serial: sequence,
                detail: outcome as u32,
                ..out
            }
        }
        Event::Released { surface, buffer } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_RELEASED,
                surface,
                serial: buffer,
                ..out
            }
        }
        Event::Pinch {
            surface,
            serial,
            phase,
            x,
            y,
            scale_65536,
            rotation_256,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_PINCH,
                surface,
                serial,
                x,
                y,
                detail: phase as u32,
                scale_65536,
                rotation_256,
                ..out
            }
        }
        Event::Scroll {
            surface,
            serial,
            dx,
            dy,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_SCROLL,
                surface,
                serial,
                x: dx,
                y: dy,
                ..out
            }
        }
        Event::Text {
            surface,
            serial,
            text,
        } => {
            let bytes = text.as_str().as_bytes();
            out.text[..bytes.len()].copy_from_slice(bytes);
            out = OxyOSEvent {
                kind: OXYOS_EVENT_TEXT,
                surface,
                serial,
                ..out
            }
        }
        Event::Preedit {
            surface,
            serial,
            text,
            cursor,
        } => {
            let bytes = text.as_str().as_bytes();
            out.text[..bytes.len()].copy_from_slice(bytes);
            out = OxyOSEvent {
                kind: OXYOS_EVENT_PREEDIT,
                surface,
                serial,
                detail: u32::from(cursor),
                ..out
            }
        }
        Event::AccessibilityAction {
            surface,
            serial,
            index,
            action,
            custom,
            start,
            end,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_ACCESSIBILITY_ACTION,
                surface,
                serial,
                detail: action as u32,
                code: index,
                width: custom,
                x: start as i32,
                y: end as i32,
                ..out
            }
        }
        // Sent only to assistive technologies, which do not use this ABI.
        Event::AccessibilityTree { .. }
        | Event::AccessibleNode { .. }
        | Event::AccessibleText { .. }
        | Event::AccessibleSelection { .. }
        | Event::AccessibilityAnnouncement { .. }
        | Event::AccessibilityCommitted { .. }
        | Event::AccessibilityRemoved { .. }
        | Event::ScreenReaderCommand { .. } => return None,
        Event::Welcome { .. } | Event::ProtocolError { .. } => return None,
        // What the clipboard holds, for the client with keyboard focus (ADR 0007).
        Event::ClipboardTypes { selection, count } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_CLIPBOARD_TYPES,
                serial: selection.into(),
                detail: count.into(),
                ..out
            }
        }
        Event::ClipboardType { selection, mime } => {
            let bytes = mime.as_str().as_bytes();
            out.text[..bytes.len()].copy_from_slice(bytes);
            out = OxyOSEvent {
                kind: OXYOS_EVENT_CLIPBOARD_TYPE,
                serial: selection.into(),
                ..out
            }
        }
        Event::ClipboardEmpty {} => out.kind = OXYOS_EVENT_CLIPBOARD_EMPTY,
        // Answered by the client library, which holds what this client copied.
        Event::ClipboardSend { .. } => return None,
        // A transaction's parts are collected by the client library; its Done completes it.
        Event::TextInputDelete { .. }
        | Event::TextInputCommitText { .. }
        | Event::TextInputPreeditText { .. }
        | Event::TextInputPreeditCursor { .. }
        | Event::TextInputPreeditSegment { .. } => return None,
        // Only to the input method, which is never a C client.
        Event::InputMethodActivate {}
        | Event::InputMethodDeactivate {}
        | Event::InputMethodPurpose { .. }
        | Event::InputMethodSurrounding { .. }
        | Event::InputMethodSelection { .. }
        | Event::InputMethodDone { .. }
        | Event::InputMethodKey { .. } => return None,
        Event::WindowState {
            window,
            workspace,
            minimized,
            focused,
            launched,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_WINDOW_STATE,
                code: window,
                detail: workspace,
                flag: u8::from(minimized) * OXYOS_WINDOW_MINIMIZED
                    + u8::from(focused) * OXYOS_WINDOW_FOCUSED
                    + u8::from(launched) * OXYOS_WINDOW_LAUNCHED,
                ..out
            }
        }
        Event::WindowTitle {
            window,
            index,
            last,
            text,
        }
        | Event::WindowAppId {
            window,
            index,
            last,
            text,
        } => {
            let bytes = text.as_str().as_bytes();
            out.text[..bytes.len()].copy_from_slice(bytes);
            out = OxyOSEvent {
                kind: if matches!(event, Event::WindowTitle { .. }) {
                    OXYOS_EVENT_WINDOW_TITLE
                } else {
                    OXYOS_EVENT_WINDOW_APP_ID
                },
                code: window,
                detail: u32::from(index),
                flag: u8::from(last),
                ..out
            }
        }
        Event::WindowClosed { window } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_WINDOW_CLOSED,
                code: window,
                ..out
            }
        }
        Event::SystemShortcut {
            shortcut,
            serial,
            output,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_SYSTEM_SHORTCUT,
                serial,
                detail: shortcut as u32,
                code: output,
                ..out
            }
        }
        Event::WindowUse {
            window,
            used,
            hidden,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_WINDOW_USE,
                code: window,
                detail: used,
                flag: u8::from(hidden),
                ..out
            }
        }
        Event::ControlLevel {
            control,
            level_65536,
            muted,
            available,
            bezel,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_CONTROL_LEVEL,
                detail: control as u32,
                code: level_65536,
                flag: (if muted { OXYOS_CONTROL_MUTED } else { 0 })
                    | (if available {
                        OXYOS_CONTROL_AVAILABLE
                    } else {
                        0
                    })
                    | (if bezel { OXYOS_CONTROL_BEZEL } else { 0 }),
                ..out
            }
        }
        Event::Power {
            battery,
            percent,
            on_ac,
            minutes,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_POWER,
                detail: battery as u32,
                code: u32::from(percent),
                width: minutes,
                flag: u8::from(on_ac),
                ..out
            }
        }
        Event::Network {
            wifi,
            signal,
            wired,
            ssid,
        } => {
            let bytes = ssid.as_str().as_bytes();
            out.text[..bytes.len()].copy_from_slice(bytes);
            out = OxyOSEvent {
                kind: OXYOS_EVENT_NETWORK,
                detail: wifi as u32,
                code: u32::from(signal),
                flag: u8::from(wired),
                ..out
            }
        }
        Event::Bluetooth { state, connected } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_BLUETOOTH,
                detail: state as u32,
                code: u32::from(connected),
                ..out
            }
        }
        Event::NotificationResult {
            serial,
            notification,
            outcome,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_NOTIFICATION_RESULT,
                code: notification,
                detail: outcome as u32,
                width: serial,
                ..out
            }
        }
        Event::NotificationText {
            notification,
            field,
            action,
            index,
            last,
            text,
        } => {
            let bytes = text.as_str().as_bytes();
            out.text[..bytes.len()].copy_from_slice(bytes);
            out = OxyOSEvent {
                kind: OXYOS_EVENT_NOTIFICATION_TEXT,
                code: notification,
                height: field as u32,
                width: action.into(),
                detail: index.into(),
                flag: last.into(),
                ..out
            }
        }
        Event::Notification {
            notification,
            banner,
            actions,
            age_s,
            withheld,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_NOTIFICATION,
                code: notification,
                width: actions.into(),
                detail: age_s,
                flag: if banner { OXYOS_NOTIFICATION_BANNER } else { 0 }
                    | if withheld {
                        OXYOS_NOTIFICATION_WITHHELD
                    } else {
                        0
                    },
                ..out
            }
        }
        Event::NotificationResponse {
            notification,
            action,
            serial,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_NOTIFICATION_RESPONSE,
                code: notification,
                width: action.into(),
                serial,
                ..out
            }
        }
        Event::NotificationRemoved { notification } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_NOTIFICATION_REMOVED,
                code: notification,
                ..out
            }
        }
        Event::NotificationPolicy {
            do_not_disturb,
            previews,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_NOTIFICATION_POLICY,
                flag: if do_not_disturb {
                    OXYOS_NOTIFICATIONS_DO_NOT_DISTURB
                } else {
                    0
                } | if previews {
                    OXYOS_NOTIFICATIONS_PREVIEWS
                } else {
                    0
                },
                ..out
            }
        }
        Event::DisplayName {
            output,
            index,
            last,
            text,
        } => {
            let bytes = text.as_str().as_bytes();
            out.text[..bytes.len()].copy_from_slice(bytes);
            out = OxyOSEvent {
                kind: OXYOS_EVENT_DISPLAY_NAME,
                code: output,
                detail: u32::from(index),
                flag: u8::from(last),
                ..out
            }
        }
        Event::DisplayMode {
            output,
            index,
            width,
            height,
            refresh_mhz,
            preferred,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_DISPLAY_MODE,
                code: output,
                detail: index,
                width,
                height,
                x: i32::try_from(refresh_mhz).unwrap_or(i32::MAX),
                flag: u8::from(preferred),
                ..out
            }
        }
        Event::DisplayScale {
            output,
            index,
            scale_120,
            recommended,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_DISPLAY_SCALE,
                code: output,
                detail: index,
                scale_120,
                flag: u8::from(recommended),
                ..out
            }
        }
        Event::Display {
            output,
            rect,
            scale_120,
            mode,
            width_mm,
            height_mm,
            primary,
            builtin,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_DISPLAY,
                code: output,
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
                scale_120,
                detail: mode,
                surface: width_mm,
                serial: u64::from(height_mm),
                flag: if primary { OXYOS_DISPLAY_PRIMARY } else { 0 }
                    | if builtin { OXYOS_DISPLAY_BUILTIN } else { 0 },
                ..out
            }
        }
        Event::DisplayRemoved { output } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_DISPLAY_REMOVED,
                code: output,
                ..out
            }
        }
        Event::DisplayConfirmation { output, seconds } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_DISPLAY_CONFIRMATION,
                code: output,
                detail: seconds,
                ..out
            }
        }
        Event::WindowSize { window, size } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_WINDOW_SIZE,
                code: window,
                width: size.width,
                height: size.height,
                ..out
            }
        }
        Event::Workspaces { count, active } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_WORKSPACES,
                detail: count,
                code: active,
                ..out
            }
        }
        Event::Output {
            output,
            rect,
            scale_120,
            refresh_mhz,
            primary,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_OUTPUT,
                code: output,
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
                scale_120,
                detail: refresh_mhz,
                flag: u8::from(primary),
                ..out
            }
        }
        Event::Refresh {
            surface,
            refresh_mhz,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_REFRESH,
                surface,
                detail: refresh_mhz,
                ..out
            }
        }
        Event::OutputRemoved { output } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_OUTPUT_REMOVED,
                code: output,
                ..out
            }
        }
        Event::CloseRequested { surface } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_CLOSE_REQUESTED,
                surface,
                ..out
            }
        }
        Event::LaunchResult { serial, launched } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_LAUNCH_RESULT,
                code: serial,
                flag: u8::from(launched),
                ..out
            }
        }
        Event::PreferenceResult { serial, accepted } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_PREFERENCE_RESULT,
                code: serial,
                flag: u8::from(accepted),
                ..out
            }
        }
        Event::Preferences {
            dark,
            reduce_motion,
            reduce_transparency,
            accent,
        } => {
            let bytes = accent.as_str().as_bytes();
            out.text[..bytes.len()].copy_from_slice(bytes);
            out = OxyOSEvent {
                kind: OXYOS_EVENT_PREFERENCES,
                flag: u8::from(dark) * OXYOS_PREFERENCES_DARK
                    + u8::from(reduce_motion) * OXYOS_PREFERENCES_REDUCE_MOTION
                    + u8::from(reduce_transparency) * OXYOS_PREFERENCES_REDUCE_TRANSPARENCY,
                ..out
            }
        }
        Event::Preference { index, last, text } => {
            let bytes = text.as_str().as_bytes();
            out.text[..bytes.len()].copy_from_slice(bytes);
            out = OxyOSEvent {
                kind: OXYOS_EVENT_PREFERENCE,
                detail: u32::from(index),
                flag: u8::from(last),
                ..out
            }
        }
        Event::Activated { surface, active } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_ACTIVATED,
                surface,
                flag: u8::from(active),
                ..out
            }
        }
        Event::MenuBar {
            window,
            serial,
            items,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_MENU_BAR,
                code: window,
                detail: serial,
                width: items,
                ..out
            }
        }
        Event::MenuBarItem {
            window,
            index,
            parent,
            kind,
            flags,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_MENU_BAR_ITEM,
                code: window,
                detail: index,
                width: parent,
                height: kind as u32,
                flag: flags.bits(),
                ..out
            }
        }
        Event::MenuBarText {
            window,
            index,
            field,
            text,
        } => {
            let bytes = text.as_str().as_bytes();
            out.text[..bytes.len()].copy_from_slice(bytes);
            out = OxyOSEvent {
                kind: OXYOS_EVENT_MENU_BAR_TEXT,
                code: window,
                detail: index,
                height: field as u32,
                ..out
            }
        }
        Event::MenuBarCommitted { window } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_MENU_BAR_COMMITTED,
                code: window,
                ..out
            }
        }
        Event::MenuItemActivated {
            surface,
            serial,
            menus,
            index,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_MENU_ITEM_ACTIVATED,
                surface,
                serial,
                code: menus,
                detail: index,
                ..out
            }
        }
        Event::CapsLock { surface, on } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_CAPS_LOCK,
                surface,
                flag: u8::from(on),
                ..out
            }
        }
        Event::LockState { locked } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_LOCK_STATE,
                flag: u8::from(locked),
                ..out
            }
        }
        Event::AuthenticationResult {
            surface,
            outcome,
            wait_s,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_AUTHENTICATION_RESULT,
                surface,
                detail: outcome as u32,
                code: wait_s,
                ..out
            }
        }
        Event::TextInputDone {
            surface,
            serial,
            commit,
        } => {
            out = OxyOSEvent {
                kind: OXYOS_EVENT_TEXT_INPUT,
                surface,
                serial,
                detail: commit,
                ..out
            }
        }
    }
    Some(out)
}

/// Converts a C positioner; `None` if an enumeration value is not the protocol's.
pub fn positioner(p: &OxyOSPositioner) -> Option<Positioner> {
    Some(Positioner {
        anchor_rect: Rect::new(p.anchor_x, p.anchor_y, p.anchor_width, p.anchor_height),
        size: Size::new(p.width, p.height),
        anchor: Anchor::from_raw(p.anchor)?,
        gravity: Gravity::from_raw(p.gravity)?,
        adjust: Adjust::from_bits(p.adjust)?,
        offset_x: p.offset_x,
        offset_y: p.offset_y,
    })
}

/// The message of this thread's last failed call, valid until its next failure.
// `no_mangle` exports a symbol the linker may collide with; the `oxyos_` prefix owns these names.
#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn oxyos_last_error() -> *const c_char {
    LAST_ERROR.with(|last| last.borrow().as_ptr())
}

/// Connects to the compositor socket at `path` and completes the handshake. Returns NULL on
/// failure (see `oxyos_last_error`).
///
/// # Safety
/// `path` is a NUL-terminated string valid for the duration of the call.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_connect(path: *const c_char) -> *mut OxyOSClient {
    if path.is_null() {
        fail("null path");
        return std::ptr::null_mut();
    }
    // SAFETY: the caller passes a valid NUL-terminated string (contract above).
    let path = unsafe { CStr::from_ptr(path) };
    let Ok(path) = path.to_str() else {
        fail("socket path is not UTF-8");
        return std::ptr::null_mut();
    };
    match Client::connect(Path::new(path)) {
        Ok(client) => Box::into_raw(Box::new(OxyOSClient {
            client,
            segments: Vec::new(),
        })),
        Err(error) => {
            fail(error);
            std::ptr::null_mut()
        }
    }
}

/// Closes the connection and frees the client. NULL is ignored.
///
/// # Safety
/// `client` is NULL or a pointer from `oxyos_client_connect` not yet closed; it is invalid after.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_close(client: *mut OxyOSClient) {
    if !client.is_null() {
        // SAFETY: the pointer came from Box::into_raw in oxyos_client_connect and is closed once.
        drop(unsafe { Box::from_raw(client) });
    }
}

/// Reborrows a caller's client pointer.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`, used by one thread at a time.
#[allow(unsafe_code)]
unsafe fn client<'a>(client: *mut OxyOSClient) -> Option<&'a mut Client> {
    // SAFETY: per the contract, a non-null pointer is a live, exclusively used OxyOSClient.
    unsafe { client.as_mut() }.map(|c| &mut c.client)
}

/// The connection's descriptor, readable when an event is queued (for poll/epoll).
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_fd(client: *mut OxyOSClient) -> c_int {
    // SAFETY: forwarded caller contract.
    match unsafe { self::client(client) } {
        Some(client) => client.as_fd().as_raw_fd(),
        None => fail("null client"),
    }
}

/// Sends one request; 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
unsafe fn send(client: *mut OxyOSClient, request: Request) -> c_int {
    // SAFETY: forwarded caller contract.
    match unsafe { self::client(client) } {
        Some(client) => status(client.send(&request)),
        None => fail("null client"),
    }
}

/// Requests a shell surface (the session's shell, with the `shell` grant): `kind` is
/// OXYOS_SHELL_BACKGROUND, _PANEL, _OVERLAY or _NOTIFICATION, `edge` OXYOS_EDGE_*, on `output`, of
/// the given logical size (a panel's thickness along its edge; a background takes the output),
/// keeping `exclusive` logical pixels of a panel's edge out of where windows are placed. The
/// compositor answers with a configure. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_create_shell_surface(
    client: *mut OxyOSClient,
    surface: u32,
    kind: u8,
    edge: u8,
    output: u32,
    width: u32,
    height: u32,
    exclusive: u32,
) -> c_int {
    let (Some(kind), Some(edge)) = (ShellKind::from_raw(kind), PanelEdge::from_raw(edge)) else {
        return fail("unknown shell surface kind or edge");
    };
    let request = Request::CreateShellSurface {
        surface,
        kind,
        edge,
        output,
        size: Size::new(width, height),
        exclusive,
    };
    // SAFETY: forwarded caller contract.
    unsafe { send(client, request) }
}

/// Requests the lock surface of `output` (the lock screen, with the `lock` grant, ADR 0010): it
/// covers the output and is all that shows while the session is locked. The compositor answers
/// with a configure. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_create_lock_surface(
    client: *mut OxyOSClient,
    surface: u32,
    output: u32,
) -> c_int {
    // SAFETY: forwarded caller contract.
    unsafe { send(client, Request::CreateLockSurface { surface, output }) }
}

/// The session's shell asks for the application `id` (`length` bytes of UTF-8, at most 128) of
/// the session's registry (ADR 0013); a LAUNCH_RESULT with `serial` answers. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `id` points to `length` bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_launch(
    client: *mut OxyOSClient,
    serial: u32,
    id: *const c_char,
    length: usize,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        return fail("null client");
    };
    if id.is_null() || length == 0 || length > 128 {
        return fail("an application id is 1 to 128 bytes");
    }
    // SAFETY: per the contract, `id` points to `length` readable bytes.
    let bytes = unsafe { std::slice::from_raw_parts(id.cast::<u8>(), length) };
    let Ok(id) = std::str::from_utf8(bytes) else {
        return fail("an application id is UTF-8");
    };
    status(client.launch(serial, id))
}

/// Settings (with the `settings` grant, ADR 0019) changes the preference `key` (NUL-terminated,
/// not empty, no `=`) to `value` (NUL-terminated, may be empty), at most 128 bytes as
/// `key=value`; a PREFERENCE_RESULT with `serial` answers. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `key` and `value` are NUL-terminated
/// strings valid for the duration of the call.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_set_preference(
    client: *mut OxyOSClient,
    serial: u32,
    key: *const c_char,
    value: *const c_char,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        return fail("null client");
    };
    if key.is_null() || value.is_null() {
        return fail("null preference");
    }
    // SAFETY: per the contract, both are NUL-terminated strings.
    let (key, value) = unsafe { (CStr::from_ptr(key), CStr::from_ptr(value)) };
    let (Ok(key), Ok(value)) = (key.to_str(), value.to_str()) else {
        return fail("a preference is UTF-8");
    };
    status(client.set_preference(serial, key, value))
}

/// Locks the session (the session's shell, with the `shell` grant). 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_lock_session(client: *mut OxyOSClient) -> c_int {
    // SAFETY: forwarded caller contract.
    unsafe { send(client, Request::LockSession {}) }
}

/// Submits the password typed in the lock surface `surface` (`length` bytes of UTF-8, at most
/// 512) for the session to check; an AUTHENTICATION_RESULT event answers. The library keeps no
/// copy: the caller overwrites its own once this returns. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `secret` points to `length` bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_authenticate(
    client: *mut OxyOSClient,
    surface: u32,
    secret: *const c_char,
    length: usize,
) -> c_int {
    // SAFETY: forwarded caller contract; no account.
    unsafe { oxyos_client_authenticate_account(client, surface, std::ptr::null(), secret, length) }
}

/// As `oxyos_client_authenticate`, for the account `account` (its Unix user's name, a
/// NUL-terminated string of at most 31 bytes; null for the session's own user): a login screen's
/// password (docs/adr/0020-oxy-account-login.mdx). 0 on success; nothing is sent on an error.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `account` is null or NUL-terminated;
/// `secret` points to `length` bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_authenticate_account(
    client: *mut OxyOSClient,
    surface: u32,
    account: *const c_char,
    secret: *const c_char,
    length: usize,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        return fail("null client");
    };
    if length > oxyos_protocol::MAX_SECRET_BYTES {
        return fail("a password is at most 512 bytes");
    }
    let bytes = if length == 0 {
        &[][..]
    } else if secret.is_null() {
        return fail("null password");
    } else {
        // SAFETY: per the contract, `secret` points to `length` readable bytes.
        unsafe { std::slice::from_raw_parts(secret.cast::<u8>(), length) }
    };
    let Ok(secret) = std::str::from_utf8(bytes) else {
        return fail("a password is UTF-8");
    };
    let account =
        if account.is_null() {
            None
        } else {
            // SAFETY: per the contract, a non-null `account` is NUL-terminated.
            let account = unsafe { CStr::from_ptr(account) };
            match account.to_str().ok().filter(|name| {
                !name.is_empty() && name.len() <= oxyos_protocol::MAX_USER_NAME_BYTES
            }) {
                Some(name) => Some(name),
                None => return fail("an account is a user name of 1 to 31 bytes of UTF-8"),
            }
        };
    if let Some(user) = account.and_then(PreeditText::new) {
        if let Err(error) = client.send(&Request::AuthenticateAccount { surface, user }) {
            return fail(error);
        }
    }
    for secret in SecretChunk::split(secret) {
        if let Err(error) = client.send(&Request::AuthenticateSecret { surface, secret }) {
            return fail(error);
        }
    }
    status(client.send(&Request::Authenticate { surface }))
}

/// Names a window (ADR 0009): its title, or with a non-zero `app_id` its application id
/// (`length` bytes of UTF-8; at most 256 bytes for a title, 128 for an id). 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `text` points to `length` bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_name_window(
    client: *mut OxyOSClient,
    surface: u32,
    app_id: u8,
    text: *const c_char,
    length: usize,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        return fail("null client");
    };
    let bytes = if length == 0 {
        &[][..]
    } else if text.is_null() {
        return fail("null text");
    } else {
        // SAFETY: per the contract, `text` points to `length` readable bytes.
        unsafe { std::slice::from_raw_parts(text.cast::<u8>(), length) }
    };
    let Ok(text) = std::str::from_utf8(bytes) else {
        return fail("a name is UTF-8");
    };
    status(client.name_window(surface, app_id != 0, text))
}

/// What the session's shell asks of windows and workspaces (ADR 0009, the `shell` grant):
/// OXYOS_MANAGE_FOCUS, _CLOSE (the application is asked), _MINIMIZE or _MOVE (to `workspace`)
/// `window`; _ACTIVATE_WORKSPACE, _ADD_WORKSPACE or _REMOVE_WORKSPACE. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_manage(
    client: *mut OxyOSClient,
    action: u8,
    window: u32,
    workspace: u32,
) -> c_int {
    let request = match action {
        OXYOS_MANAGE_FOCUS => Request::FocusWindow { window },
        OXYOS_MANAGE_CLOSE => Request::CloseWindow { window },
        OXYOS_MANAGE_MINIMIZE => Request::MinimizeWindow { window },
        OXYOS_MANAGE_MOVE => Request::MoveWindowToWorkspace { window, workspace },
        OXYOS_MANAGE_ACTIVATE_WORKSPACE => Request::ActivateWorkspace { workspace },
        OXYOS_MANAGE_ADD_WORKSPACE => Request::AddWorkspace {},
        OXYOS_MANAGE_REMOVE_WORKSPACE => Request::RemoveWorkspace { workspace },
        OXYOS_MANAGE_HIDE_APPLICATION => Request::HideApplication { window },
        OXYOS_MANAGE_QUIT_APPLICATION => Request::QuitApplication { window },
        _ => return fail("unknown action"),
    };
    // SAFETY: forwarded caller contract.
    unsafe { send(client, request) }
}

/// A window a shell surface shows live (docs/adr/0022-window-views.mdx): the window's number,
/// where it is drawn in the surface (logical pixels; fitted with its proportions kept, centred),
/// the part of the surface it may cover, its corners' radius and its opacity (255: as it is).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OxyOSWindowView {
    pub window: u32,
    pub rect: OxyOSRect,
    pub clip: OxyOSRect,
    pub radius: u32,
    pub opacity: u8,
}

/// The windows the shell's surface `surface` shows live, bottom to top, replacing those it
/// showed: `count` (at most 64) `OxyOSWindowView`s at `views` (any pointer when 0), shown with
/// the surface's next present. The compositor draws them over the surface's pixels; the shell
/// never receives them. Only the session's shell may (the `shell` grant). 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `views` points to `count` readable
/// `OxyOSWindowView`s when `count` is not 0.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_show_windows(
    client: *mut OxyOSClient,
    surface: u32,
    views: *const OxyOSWindowView,
    count: usize,
) -> c_int {
    if count > oxyos_protocol::MAX_WINDOW_VIEWS {
        return fail("too many windows shown live");
    }
    let views = if count == 0 {
        &[][..]
    } else {
        // SAFETY: per the contract, `views` points to `count` readable views.
        unsafe { std::slice::from_raw_parts(views, count) }
    };
    let rect = |r: OxyOSRect| Rect::new(r.x, r.y, r.width, r.height);
    // SAFETY: forwarded caller contract.
    let cleared = unsafe { send(client, Request::ClearWindowViews { surface }) };
    if cleared != 0 {
        return cleared;
    }
    for view in views {
        let request = Request::PlaceWindowView {
            surface,
            window: view.window,
            rect: rect(view.rect),
            clip: rect(view.clip),
            radius: view.radius,
            opacity: view.opacity,
        };
        // SAFETY: forwarded caller contract.
        let sent = unsafe { send(client, request) };
        if sent != 0 {
            return sent;
        }
    }
    0
}

/// The pointer's shape while it is over `surface`: one of OXYOS_POINTER_*. 0 on success, or
/// OXYOS_ERROR for a shape that is not one.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_set_pointer_shape(
    client: *mut OxyOSClient,
    surface: u32,
    shape: u8,
) -> c_int {
    let Some(shape) = oxyos_protocol::PointerShape::from_raw(shape) else {
        return fail("not a pointer shape");
    };
    // SAFETY: forwarded caller contract.
    unsafe { send(client, Request::SetPointerShape { surface, shape }) }
}

/// Moves a toplevel with the pointer, from the press `serial` names (the press being handled),
/// until its buttons are up: the compositor moves it, without the client (ADR 0016). 0 on
/// success; a press that is not the latest, or not held, is ignored.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_begin_move(
    client: *mut OxyOSClient,
    surface: u32,
    serial: u64,
) -> c_int {
    // SAFETY: forwarded caller contract.
    unsafe { send(client, Request::BeginMove { surface, serial }) }
}

/// Makes a toplevel fill its output, or leave it. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_set_fullscreen(
    client: *mut OxyOSClient,
    surface: u32,
    enabled: bool,
) -> c_int {
    // SAFETY: forwarded caller contract.
    unsafe { send(client, Request::SetFullscreen { surface, enabled }) }
}

/// Zooms a toplevel to fill its output's work area, or restores the frame it had (ADR 0016).
/// 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_set_maximized(
    client: *mut OxyOSClient,
    surface: u32,
    enabled: bool,
) -> c_int {
    // SAFETY: forwarded caller contract.
    unsafe { send(client, Request::SetMaximized { surface, enabled }) }
}

/// Minimizes a toplevel, as the shell does: into the dock (ADR 0016). 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_minimize(client: *mut OxyOSClient, surface: u32) -> c_int {
    // SAFETY: forwarded caller contract.
    unsafe { send(client, Request::Minimize { surface }) }
}

/// The logical sizes a toplevel can take; a zero in a maximum is no bound. The compositor never
/// configures it outside them (ADR 0016). 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_set_size_limits(
    client: *mut OxyOSClient,
    surface: u32,
    min_width: u32,
    min_height: u32,
    max_width: u32,
    max_height: u32,
) -> c_int {
    let request = Request::SetSizeLimits {
        surface,
        min_width,
        min_height,
        max_width,
        max_height,
    };
    // SAFETY: forwarded caller contract.
    unsafe { send(client, request) }
}

/// Limits where a surface takes pointer and touch input to a surface-local logical rectangle
/// (an empty one: nowhere); input elsewhere reaches what is below. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_set_input_region(
    client: *mut OxyOSClient,
    surface: u32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> c_int {
    let region = Rect::new(x, y, width, height);
    // SAFETY: forwarded caller contract.
    unsafe { send(client, Request::SetInputRegion { surface, region }) }
}

/// Requests a toplevel window of the given logical size; the compositor answers with a
/// configure. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_create_toplevel(
    client: *mut OxyOSClient,
    surface: u32,
    width: u32,
    height: u32,
) -> c_int {
    let size = Size::new(width, height);
    // SAFETY: forwarded caller contract.
    unsafe { send(client, Request::CreateToplevel { surface, size }) }
}

/// Requests a popup placed relative to `parent` by `positioner`. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `positioner` points to a valid
/// `OxyOSPositioner` for the duration of the call.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_create_popup(
    client: *mut OxyOSClient,
    surface: u32,
    parent: u32,
    positioner: *const OxyOSPositioner,
) -> c_int {
    // SAFETY: the caller passes a valid positioner or NULL.
    let Some(positioner) = unsafe { positioner.as_ref() }.and_then(self::positioner) else {
        return fail("invalid positioner");
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        send(
            client,
            Request::CreatePopup {
                surface,
                parent,
                positioner,
            },
        )
    }
}

/// Places an existing popup again (its anchor moved or its content resized); the compositor
/// answers with a configure. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `positioner` points to a valid
/// `OxyOSPositioner` for the duration of the call.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_reposition(
    client: *mut OxyOSClient,
    surface: u32,
    positioner: *const OxyOSPositioner,
) -> c_int {
    // SAFETY: the caller passes a valid positioner or NULL.
    let Some(positioner) = unsafe { positioner.as_ref() }.and_then(self::positioner) else {
        return fail("invalid positioner");
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        send(
            client,
            Request::Reposition {
                surface,
                positioner,
            },
        )
    }
}

/// Publishes a surface's accessibility snapshot (ADR 0003). 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `nodes` points to `count` nodes (or is
/// NULL with `count` 0), whose `name`, `value` and `custom_actions` are NULL or NUL-terminated
/// strings.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_publish_accessibility(
    client: *mut OxyOSClient,
    surface: u32,
    nodes: *const OxyOSAccessibleNode,
    count: usize,
) -> c_int {
    let nodes: &[OxyOSAccessibleNode] = if count == 0 {
        &[]
    } else if nodes.is_null() {
        return fail("null nodes");
    } else {
        // SAFETY: the caller passes `count` valid nodes.
        unsafe { std::slice::from_raw_parts(nodes, count) }
    };
    let text = |pointer: *const c_char| -> Option<&str> {
        if pointer.is_null() {
            Some("")
        } else {
            // SAFETY: the caller passes NULL or NUL-terminated strings that outlive the call.
            unsafe { CStr::from_ptr(pointer) }.to_str().ok()
        }
    };
    let mut accessible = Vec::with_capacity(nodes.len());
    for node in nodes {
        let (Some(role), Some(state), Some(actions), Some(name), Some(value), Some(custom)) = (
            AccessibleRole::from_raw(node.role),
            AccessibleState::new(node.state),
            AccessibleActions::new(node.actions),
            text(node.name),
            text(node.value),
            text(node.custom_actions),
        ) else {
            return fail("invalid accessible node");
        };
        accessible.push(Accessible {
            parent: node.parent,
            role,
            state,
            actions,
            rect: Rect::new(node.x, node.y, node.width, node.height),
            name,
            value,
            custom,
            selection: (node.has_selection != 0)
                .then_some((node.selection_start, node.selection_end)),
        });
    }
    // SAFETY: forwarded caller contract.
    match unsafe { self::client(client) } {
        Some(client) => status(client.publish_accessibility(surface, &accessible)),
        None => fail("null client"),
    }
}

/// Declares a toplevel's menus (ADR 0021), replacing its last: `count` entries in order, each
/// parent before what it holds, as the commit `serial` (above every earlier one of the window;
/// a MENU_ITEM_ACTIVATED names it with the chosen entry's index). A label longer than the
/// protocol allows is cut at a character; a shortcut longer is left out, since a cut chord would
/// be another. More entries, an unknown kind or flags: OXYOS_ERROR, and nothing is sent. 0 on
/// success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `items` points to `count` entries (or
/// is NULL with `count` 0), whose `label` and `shortcut` are NULL or NUL-terminated strings.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_set_menus(
    client: *mut OxyOSClient,
    surface: u32,
    serial: u32,
    items: *const OxyOSMenuItem,
    count: usize,
) -> c_int {
    let items: &[OxyOSMenuItem] = if count == 0 {
        &[]
    } else if items.is_null() {
        return fail("null menu items");
    } else {
        // SAFETY: the caller passes `count` valid entries.
        unsafe { std::slice::from_raw_parts(items, count) }
    };
    let text = |pointer: *const c_char| -> Option<&str> {
        if pointer.is_null() {
            Some("")
        } else {
            // SAFETY: the caller passes NULL or NUL-terminated strings that outlive the call.
            unsafe { CStr::from_ptr(pointer) }.to_str().ok()
        }
    };
    let mut menus = Vec::with_capacity(items.len());
    for item in items {
        let (Some(kind), Some(flags), Some(label), Some(shortcut)) = (
            MenuKind::from_raw(item.kind),
            MenuFlags::new(item.flags),
            text(item.label),
            text(item.shortcut),
        ) else {
            return fail("invalid menu entry");
        };
        menus.push(Menu {
            parent: item.parent,
            kind,
            flags,
            label,
            shortcut,
        });
    }
    // SAFETY: forwarded caller contract.
    match unsafe { self::client(client) } {
        Some(client) => status(client.set_menus(surface, serial, &menus)),
        None => fail("null client"),
    }
}

/// The session's shell chooses the entry `index` of the commit `serial` of window `window`'s
/// menus, as a MENU_BAR told them (ADR 0021). The compositor sends it on only if they are still
/// the active window's. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_activate_menu_item(
    client: *mut OxyOSClient,
    window: u32,
    serial: u32,
    index: u32,
) -> c_int {
    // SAFETY: forwarded caller contract.
    unsafe {
        send(
            client,
            Request::ActivateMenuItem {
                window,
                serial,
                index,
            },
        )
    }
}

/// Posts a notification as `serial` (docs/adr/0026-notifications.mdx): a NOTIFICATION_RESULT with
/// it answers. Texts longer than the protocol allows are cut at a character; more than 3 actions,
/// data longer than 4096 bytes, an identifier longer than 64 or an action without an id or a label
/// is OXYOS_ERROR, and nothing is sent. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `notification` points to a valid
/// OxyOSNotification whose strings are NULL or NUL-terminated and whose `actions` point to
/// `action_count` entries (or NULL with 0), all valid for the duration of the call.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_post_notification(
    client: *mut OxyOSClient,
    serial: u32,
    notification: *const OxyOSNotification,
) -> c_int {
    if notification.is_null() {
        return fail("null notification");
    }
    // SAFETY: per the contract, a valid notification.
    let notification = unsafe { &*notification };
    let text = |pointer: *const c_char| -> Option<&str> {
        if pointer.is_null() {
            Some("")
        } else {
            // SAFETY: the caller passes NULL or NUL-terminated strings that outlive the call.
            unsafe { CStr::from_ptr(pointer) }.to_str().ok()
        }
    };
    let actions: &[OxyOSNotificationAction] = if notification.action_count == 0 {
        &[]
    } else if notification.actions.is_null() {
        return fail("null notification actions");
    } else {
        // SAFETY: the caller passes `action_count` valid entries.
        unsafe { std::slice::from_raw_parts(notification.actions, notification.action_count) }
    };
    let mut posted = Vec::with_capacity(actions.len());
    for action in actions {
        let (Some(id), Some(label)) = (text(action.id), text(action.label)) else {
            return fail("a notification's action is UTF-8");
        };
        if id.is_empty() || label.is_empty() {
            return fail("a notification's action has an id and a label");
        }
        let reply = if action.reply.is_null() {
            None
        } else {
            match text(action.reply) {
                Some(reply) => Some(reply),
                None => return fail("a notification's action is UTF-8"),
            }
        };
        posted.push(NotificationActionPost {
            id,
            label,
            reply,
            background: action.background,
        });
    }
    let (Some(identifier), Some(title), Some(subtitle), Some(body), Some(data)) = (
        text(notification.identifier),
        text(notification.title),
        text(notification.subtitle),
        text(notification.body),
        text(notification.data),
    ) else {
        return fail("a notification is UTF-8");
    };
    let post = NotificationPost {
        identifier,
        title,
        subtitle,
        body,
        data,
        actions: &posted,
        banner: notification.banner,
    };
    // SAFETY: forwarded caller contract.
    match unsafe { self::client(client) } {
        Some(client) => status(client.post_notification(serial, &post)),
        None => fail("null client"),
    }
}

/// Withdraws one of the application's notifications by the session's number for it (0: every one
/// of its own); from the session's shell, any (0: all). 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_withdraw_notification(
    client: *mut OxyOSClient,
    notification: u32,
) -> c_int {
    // SAFETY: forwarded caller contract.
    unsafe { send(client, Request::WithdrawNotification { notification }) }
}

/// The session's shell turns Do Not Disturb on or off from its Control Center
/// (docs/adr/0026-notifications.mdx): a change of the user's `do_not_disturb` preference,
/// answered by a PREFERENCE_RESULT with `serial`, as `oxyos_client_set_appearance`. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_set_do_not_disturb(
    client: *mut OxyOSClient,
    serial: u32,
    enabled: bool,
) -> c_int {
    // SAFETY: forwarded caller contract.
    unsafe { send(client, Request::SetDoNotDisturb { serial, enabled }) }
}

/// The session's shell activates the notification (`action` 0) or its action `action` (from 1),
/// citing `serial`, an input it received; `reply` is the text typed for a reply action (`length`
/// bytes of UTF-8, cut at a character past 1024; NULL or 0 for none). 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `reply` is NULL or points to `length`
/// readable bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_activate_notification(
    client: *mut OxyOSClient,
    notification: u32,
    action: u8,
    serial: u64,
    reply: *const c_char,
    length: usize,
) -> c_int {
    let reply = if reply.is_null() || length == 0 {
        ""
    } else {
        // SAFETY: per the contract, `reply` points to `length` readable bytes.
        let bytes = unsafe { std::slice::from_raw_parts(reply.cast::<u8>(), length) };
        match std::str::from_utf8(bytes) {
            Ok(reply) => reply,
            Err(_) => return fail("a reply is UTF-8"),
        }
    };
    // SAFETY: forwarded caller contract.
    match unsafe { self::client(client) } {
        Some(client) => status(client.activate_notification(notification, action, serial, reply)),
        None => fail("null client"),
    }
}

/// The session's shell gives the keyboard focus to one of its own panels or overlays (ADR 0024),
/// citing `serial`, an input it received (a press, a key, a SYSTEM_SHORTCUT); one it did not
/// receive lately is ignored. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_focus_shell_surface(
    client: *mut OxyOSClient,
    surface: u32,
    serial: u64,
) -> c_int {
    // SAFETY: forwarded caller contract.
    unsafe { send(client, Request::FocusShellSurface { surface, serial }) }
}

/// The session's shell's Control Center (ADR 0025): sets a level (`control`: OXYOS_CONTROL_*;
/// `level`: 0 to 65536). The session applies it and tells the level then (CONTROL_LEVEL). 0 on
/// success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_set_control_level(
    client: *mut OxyOSClient,
    control: u8,
    level: u32,
) -> c_int {
    let Some(control) = Control::from_raw(control) else {
        return fail("no such control");
    };
    if level > FULL_LEVEL {
        return fail("a level is at most 65536");
    }
    // SAFETY: forwarded caller contract.
    unsafe {
        send(
            client,
            Request::SetControlLevel {
                control,
                level_65536: level,
            },
        )
    }
}

/// Mutes a volume (`control`: OXYOS_CONTROL_OUTPUT_VOLUME or _INPUT_VOLUME) or unmutes it. 0 on
/// success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_set_control_muted(
    client: *mut OxyOSClient,
    control: u8,
    muted: bool,
) -> c_int {
    let Some(control) = Control::from_raw(control) else {
        return fail("no such control");
    };
    // SAFETY: forwarded caller contract.
    unsafe { send(client, Request::SetControlMuted { control, muted }) }
}

/// Turns Wi-Fi on or off (the session's shell, ADR 0025). 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_set_wifi_enabled(
    client: *mut OxyOSClient,
    enabled: bool,
) -> c_int {
    // SAFETY: forwarded caller contract.
    unsafe { send(client, Request::SetWifiEnabled { enabled }) }
}

/// Sets the appearance light or dark (the session's shell, ADR 0025): a change of the user's
/// `appearance` preference, answered by a PREFERENCE_RESULT with `serial`. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_set_appearance(
    client: *mut OxyOSClient,
    serial: u32,
    dark: bool,
) -> c_int {
    // SAFETY: forwarded caller contract.
    unsafe { send(client, Request::SetAppearance { serial, dark }) }
}

/// Settings (its `settings` grant, ADR 0018) shows the display of `output` at one of its modes
/// (`width`×`height` at `refresh_mhz`, as a DISPLAY_MODE told it), on trial: it goes back unless
/// `oxyos_client_confirm_displays` keeps it within the seconds a DISPLAY_CONFIRMATION says. A
/// PREFERENCE_RESULT with `serial` answers. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_set_display_mode(
    client: *mut OxyOSClient,
    serial: u32,
    output: u32,
    width: u32,
    height: u32,
    refresh_mhz: u32,
) -> c_int {
    // SAFETY: forwarded caller contract.
    unsafe {
        send(
            client,
            Request::SetDisplayMode {
                serial,
                output,
                width,
                height,
                refresh_mhz,
            },
        )
    }
}

/// Settings shows the display of `output` at one of its scale choices (in 120ths), kept at once.
/// A PREFERENCE_RESULT with `serial` answers. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_set_display_scale(
    client: *mut OxyOSClient,
    serial: u32,
    output: u32,
    scale_120: u32,
) -> c_int {
    // SAFETY: forwarded caller contract.
    unsafe {
        send(
            client,
            Request::SetDisplayScale {
                serial,
                output,
                scale_120,
            },
        )
    }
}

/// Settings wants the display of `output` at (`x`, `y`) in the global space (logical pixels): the
/// session places it beside the others, touching one, and keeps the arrangement. A
/// PREFERENCE_RESULT with `serial` answers. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_set_display_origin(
    client: *mut OxyOSClient,
    serial: u32,
    output: u32,
    x: i32,
    y: i32,
) -> c_int {
    // SAFETY: forwarded caller contract.
    unsafe {
        send(
            client,
            Request::SetDisplayOrigin {
                serial,
                output,
                x,
                y,
            },
        )
    }
}

/// Settings makes the display of `output` the primary (it has the dock), kept at once. A
/// PREFERENCE_RESULT with `serial` answers. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_set_primary_display(
    client: *mut OxyOSClient,
    serial: u32,
    output: u32,
) -> c_int {
    // SAFETY: forwarded caller contract.
    unsafe { send(client, Request::SetPrimaryDisplay { serial, output }) }
}

/// Settings keeps the mode on trial (`keep`), or has the display go back now. A PREFERENCE_RESULT
/// with `serial` answers. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_confirm_displays(
    client: *mut OxyOSClient,
    serial: u32,
    keep: bool,
) -> c_int {
    // SAFETY: forwarded caller contract.
    unsafe { send(client, Request::ConfirmDisplays { serial, keep }) }
}

/// The Control Center opened: the session looks at the system again now (ADR 0025). 0 on
/// success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_refresh_controls(client: *mut OxyOSClient) -> c_int {
    // SAFETY: forwarded caller contract.
    unsafe { send(client, Request::RefreshControls {}) }
}

/// Announces `length` bytes of UTF-8 to assistive technologies (ADR 0003): `politeness` is
/// OXYOS_POLITE or OXYOS_ASSERTIVE. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `text` points to `length` bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_announce(
    client: *mut OxyOSClient,
    surface: u32,
    politeness: u8,
    text: *const c_char,
    length: usize,
) -> c_int {
    let Some(politeness) = Politeness::from_raw(politeness) else {
        return fail("not a politeness");
    };
    let bytes = if length == 0 {
        &[][..]
    } else if text.is_null() {
        return fail("null text");
    } else {
        // SAFETY: per the contract, `text` points to `length` readable bytes.
        unsafe { std::slice::from_raw_parts(text.cast::<u8>(), length) }
    };
    let Ok(text) = std::str::from_utf8(bytes) else {
        return fail("an announcement is UTF-8");
    };
    // SAFETY: forwarded caller contract.
    match unsafe { self::client(client) } {
        Some(client) => status(client.announce(surface, politeness, text)),
        None => fail("null client"),
    }
}

/// Asks for the popup's input grab, citing the serial of the input event that opened it.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_grab_popup(
    client: *mut OxyOSClient,
    surface: u32,
    serial: u64,
) -> c_int {
    // SAFETY: forwarded caller contract.
    unsafe { send(client, Request::GrabPopup { surface, serial }) }
}

/// Destroys a surface and the buffers it drew with. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_destroy_surface(
    client: *mut OxyOSClient,
    surface: u32,
) -> c_int {
    // SAFETY: forwarded caller contract.
    match unsafe { self::client(client) } {
        Some(client) => status(client.destroy_surface(surface)),
        None => fail("null client"),
    }
}

/// Presents `pixels` (rows of `width * 4` bytes, `height` rows) on `surface` in answer to the
/// configure with serial `configure`. `format` is a DRM fourcc: `OXYOS_FORMAT_XRGB8888` (opaque,
/// little-endian B, G, R, X) or `OXYOS_FORMAT_ARGB8888` (premultiplied B, G, R, A). Returns the
/// frame's sequence number, or -1 on failure.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `pixels` points to `length` readable
/// bytes for the duration of the call.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_present(
    client: *mut OxyOSClient,
    surface: u32,
    configure: u64,
    width: u32,
    height: u32,
    format: u32,
    pixels: *const u8,
    length: usize,
) -> i64 {
    if pixels.is_null() {
        fail("null pixels");
        return -1;
    }
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        fail("null client");
        return -1;
    };
    // SAFETY: the caller guarantees `length` readable bytes at `pixels`.
    let pixels = unsafe { std::slice::from_raw_parts(pixels, length) };
    let Some(format) = Format::from_raw(format) else {
        fail("unknown pixel format");
        return -1;
    };
    match client.present_format(surface, configure, Size::new(width, height), format, pixels) {
        Ok(sequence) => i64::try_from(sequence).unwrap_or(-1),
        Err(error) => {
            fail(error);
            -1
        }
    }
}

/// A rectangle of a buffer, in its pixels: what changed in a frame (`oxyos_client_present_damaged`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OxyOSRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Copies `length` bytes of UTF-8 text at `text` as the clipboard (`text/plain;charset=utf-8`),
/// with the serial of the user input that asked for it (ADR 0007). The client library keeps the
/// text and writes it into each paste itself. 0 on success; the compositor may still refuse the
/// copy (a serial that is not a recent input of this client, or keyboard focus lost).
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `text` points to `length` bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_copy_text(
    client: *mut OxyOSClient,
    serial: u64,
    text: *const c_char,
    length: usize,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        return fail("null client");
    };
    if text.is_null() && length > 0 {
        return fail("null text");
    }
    let bytes = if length == 0 {
        Vec::new()
    } else {
        // SAFETY: per the contract, `text` points to `length` readable bytes.
        unsafe { std::slice::from_raw_parts(text.cast::<u8>(), length) }.to_vec()
    };
    if std::str::from_utf8(&bytes).is_err() {
        return fail("text is not UTF-8");
    }
    let mime = TextChunk::new(CLIPBOARD_TEXT).expect("a short type");
    status(client.copy(serial, vec![(mime, bytes)]))
}

/// Empties the clipboard if this client's copy is what it holds. 0 on success.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_clear_clipboard(
    client: *mut OxyOSClient,
    serial: u64,
) -> c_int {
    // SAFETY: forwarded caller contract.
    match unsafe { self::client(client) } {
        Some(client) => status(client.clear_clipboard(serial)),
        None => fail("null client"),
    }
}

/// Pastes `mime` (`mime_length` bytes, at most 32) of the clipboard content the compositor
/// named `selection` (a CLIPBOARD_TYPES event's serial), with the serial of the user input that
/// asked for it. Returns the read end of a pipe the content arrives through, which the caller
/// reads to its end and closes; it ends at once, empty, if the paste was refused. OXYOS_ERROR on
/// failure.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `mime` points to `mime_length` bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_paste(
    client: *mut OxyOSClient,
    selection: u32,
    mime: *const c_char,
    mime_length: usize,
    serial: u64,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        return fail("null client");
    };
    if mime.is_null() {
        return fail("null type");
    }
    // SAFETY: per the contract, `mime` points to `mime_length` readable bytes.
    let bytes = unsafe { std::slice::from_raw_parts(mime.cast::<u8>(), mime_length) };
    let Some(mime) = std::str::from_utf8(bytes).ok().and_then(TextChunk::new) else {
        return fail("a type is UTF-8 of 1 to 32 bytes");
    };
    match client.receive_clipboard(selection, mime, serial) {
        Ok(pipe) => std::os::fd::IntoRawFd::into_raw_fd(pipe),
        Err(error) => fail(error),
    }
}

/// [`oxyos_client_present`] for a frame of which only `count` rectangles at `damage` changed
/// since the surface's previous frame (none: everything). The compositor hears the damage, and
/// a reused buffer receives only what changed since it was last written
/// (docs/adr/0005-damage.mdx).
///
/// # Safety
/// As [`oxyos_client_present`]; `damage` points to `count` readable `OxyOSRect`s (any pointer when
/// `count` is 0).
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_present_damaged(
    client: *mut OxyOSClient,
    surface: u32,
    configure: u64,
    width: u32,
    height: u32,
    format: u32,
    pixels: *const u8,
    length: usize,
    damage: *const OxyOSRect,
    count: usize,
) -> i64 {
    if pixels.is_null() || (count > 0 && damage.is_null()) {
        fail("null pixels or damage");
        return -1;
    }
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        fail("null client");
        return -1;
    };
    // SAFETY: the caller guarantees `length` readable bytes at `pixels`.
    let pixels = unsafe { std::slice::from_raw_parts(pixels, length) };
    let damage: Vec<Rect> = if count == 0 {
        Vec::new()
    } else {
        // SAFETY: the caller guarantees `count` readable rectangles at `damage`.
        unsafe { std::slice::from_raw_parts(damage, count) }
            .iter()
            .map(|rect| Rect::new(rect.x, rect.y, rect.width, rect.height))
            .collect()
    };
    let Some(format) = Format::from_raw(format) else {
        fail("unknown pixel format");
        return -1;
    };
    let damage = (!damage.is_empty()).then_some(damage.as_slice());
    match client.present_damaged(
        surface,
        configure,
        Size::new(width, height),
        format,
        pixels,
        damage,
    ) {
        Ok(sequence) => i64::try_from(sequence).unwrap_or(-1),
        Err(error) => {
            fail(error);
            -1
        }
    }
}

/// The connection takes each key's text with the key (docs/adr/0029-chromium-on-oxyos.mdx): a
/// KEY event then carries what the session's layout typed with it in `text`, and that text is
/// never sent again as TEXT or in a text input transaction. 0, or OXYOS_ERROR.
///
/// # Safety
/// `client` is null or came from [`oxyos_client_connect`] and is not used by another thread.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_set_key_text(
    client: *mut OxyOSClient,
    enabled: bool,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        fail("null client");
        return OXYOS_ERROR;
    };
    status(client.send(&Request::SetKeyText { enabled }))
}

/// The rectangles a caller passed (none when `count` is 0), as the protocol's.
///
/// # Safety
/// `damage` points to `count` readable `OxyOSRect`s (any pointer when `count` is 0).
#[allow(unsafe_code)]
unsafe fn rects(damage: *const OxyOSRect, count: usize) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    // SAFETY: the caller guarantees `count` readable rectangles at `damage`.
    unsafe { std::slice::from_raw_parts(damage, count) }
        .iter()
        .map(|rect| Rect::new(rect.x, rect.y, rect.width, rect.height))
        .collect()
}

/// Makes shared memory the caller owns a buffer of the connection: `fd` (a regular file sealed
/// against shrinking, as a memfd, holding `height` rows of `stride` bytes from `offset`) in
/// `format`, of `width` × `height` pixels. Returns the buffer's id (above 0), or -1. The caller
/// keeps `fd` and draws into it; the compositor reads it from each
/// [`oxyos_client_submit_buffer`] naming it until a RELEASED event names it.
///
/// # Safety
/// `client` is null or came from [`oxyos_client_connect`] and is not used by another thread;
/// `fd` is an open descriptor for the duration of the call.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_attach_buffer(
    client: *mut OxyOSClient,
    fd: c_int,
    format: u32,
    width: u32,
    height: u32,
    stride: u32,
    offset: u32,
) -> i64 {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        fail("null client");
        return -1;
    };
    let Some(format) = Format::from_raw(format) else {
        fail("unknown pixel format");
        return -1;
    };
    if fd < 0 {
        fail("no descriptor");
        return -1;
    }
    // SAFETY: the caller guarantees `fd` is open for the duration of the call.
    let fd = unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) };
    let layout = oxyos_protocol::BufferLayout {
        format,
        extent: Size::new(width, height),
        stride,
        offset,
    };
    match client.attach_buffer(fd, layout) {
        Ok(buffer) => i64::try_from(buffer).unwrap_or(-1),
        Err(error) => {
            fail(error);
            -1
        }
    }
}

/// Submits a buffer from [`oxyos_client_attach_buffer`] for `surface` in answer to the configure
/// with serial `configure`, of which only `count` rectangles at `damage` changed since the
/// surface's previous frame (none: everything). Returns the frame's sequence number, which its
/// FEEDBACK names, or -1.
///
/// # Safety
/// As [`oxyos_client_attach_buffer`]; `damage` points to `count` readable `OxyOSRect`s (any
/// pointer when `count` is 0).
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_submit_buffer(
    client: *mut OxyOSClient,
    surface: u32,
    buffer: u64,
    configure: u64,
    damage: *const OxyOSRect,
    count: usize,
) -> i64 {
    if count > 0 && damage.is_null() {
        fail("null damage");
        return -1;
    }
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        fail("null client");
        return -1;
    };
    // SAFETY: forwarded caller contract.
    let damage = unsafe { rects(damage, count) };
    let damage = (!damage.is_empty()).then_some(damage.as_slice());
    match client.submit_buffer(surface, buffer, configure, damage) {
        Ok(sequence) => i64::try_from(sequence).unwrap_or(-1),
        Err(error) => {
            fail(error);
            -1
        }
    }
}

/// Ends a buffer from [`oxyos_client_attach_buffer`]; the compositor forgets it once nothing of
/// it is shown. 0, or OXYOS_ERROR.
///
/// # Safety
/// As [`oxyos_client_attach_buffer`].
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_detach_buffer(
    client: *mut OxyOSClient,
    buffer: u64,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        fail("null client");
        return OXYOS_ERROR;
    };
    status(client.detach_buffer(buffer))
}

/// Takes one queued event without blocking: `OXYOS_POLL_EVENT` with `*out` filled,
/// `OXYOS_POLL_EMPTY`, `OXYOS_POLL_CLOSED` once the compositor closed the connection, or `OXYOS_ERROR`
/// (a protocol error ends the connection).
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `out` points to a writable `OxyOSEvent`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_poll(
    client: *mut OxyOSClient,
    out: *mut OxyOSEvent,
) -> c_int {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        return fail("null client");
    };
    // SAFETY: the caller passes a writable OxyOSEvent or NULL.
    let Some(out) = (unsafe { out.as_mut() }) else {
        return fail("null event");
    };
    loop {
        match client.poll() {
            Ok(Poll::Event(event)) => {
                if let Some(event) = convert(&event) {
                    *out = event;
                    return OXYOS_POLL_EVENT;
                }
            }
            Ok(Poll::Empty) => return OXYOS_POLL_EMPTY,
            Ok(Poll::Closed) => return OXYOS_POLL_CLOSED,
            Err(error) => return fail(error),
        }
    }
}

/// Tells the compositor about the text field focused in `surface` (ADR 0008), or that none is
/// (`field` NULL): its purpose (OXYOS_PURPOSE_*), hints (OXYOS_HINT_* bits), its text (`length`
/// bytes of UTF-8, the composition left out) with the selection's byte offsets in it, and its
/// caret's rectangle in surface coordinates if `has_caret`. Only what changed is sent. A
/// non-zero `new_field` describes another field than the one told before. Returns the commit
/// serial transactions for this state carry (0 once disabled), or -1 on failure.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `field` is NULL or points to a
/// valid `OxyOSTextField` whose `text` points to `length` bytes.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_text_input(
    client: *mut OxyOSClient,
    surface: u32,
    field: *const OxyOSTextField,
    new_field: u8,
) -> i64 {
    // SAFETY: forwarded caller contract.
    let Some(client) = (unsafe { self::client(client) }) else {
        fail("null client");
        return -1;
    };
    // SAFETY: the caller passes NULL or a valid field.
    let result = match unsafe { field.as_ref() } {
        None => client.text_input(surface, None, new_field != 0),
        Some(field) => {
            let bytes = if field.length == 0 {
                &[][..]
            } else if field.text.is_null() {
                fail("null text");
                return -1;
            } else {
                // SAFETY: per the contract, `text` points to `length` readable bytes.
                unsafe { std::slice::from_raw_parts(field.text.cast::<u8>(), field.length) }
            };
            let (Some(purpose), Some(hints), Ok(text)) = (
                TextPurpose::from_raw(field.purpose),
                TextHints::new(field.hints),
                std::str::from_utf8(bytes),
            ) else {
                fail("invalid purpose, hints or text");
                return -1;
            };
            let (anchor, cursor) = (field.anchor as usize, field.cursor as usize);
            if !text.is_char_boundary(anchor) || !text.is_char_boundary(cursor) {
                fail("selection outside the text or inside a character");
                return -1;
            }
            let caret = (field.has_caret != 0).then(|| {
                Rect::new(
                    field.caret_x,
                    field.caret_y,
                    field.caret_width,
                    field.caret_height,
                )
            });
            let field = Field {
                purpose,
                hints,
                text,
                anchor,
                cursor,
                caret,
            };
            client.text_input(surface, Some(&field), new_field != 0)
        }
    };
    match result {
        Ok(serial) => serial.into(),
        Err(error) => {
            fail(error);
            -1
        }
    }
}

/// Fills `*out` with the transaction the TEXT_INPUT event just returned completed: 1, or 0 if
/// the last event was not one. Its strings (not NUL-terminated) and segments stay valid until the
/// next `oxyos_client_poll`.
///
/// # Safety
/// `client` is a live pointer from `oxyos_client_connect`; `out` points to a writable
/// `OxyOSTextTransaction`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_client_text_transaction(
    client: *mut OxyOSClient,
    out: *mut OxyOSTextTransaction,
) -> c_int {
    // SAFETY: per the contract, a non-null pointer is a live, exclusively used OxyOSClient.
    let Some(wrapper) = (unsafe { client.as_mut() }) else {
        return fail("null client");
    };
    // SAFETY: the caller passes a writable transaction or NULL.
    let Some(out) = (unsafe { out.as_mut() }) else {
        return fail("null transaction");
    };
    let Some(transaction) = wrapper.client.transaction() else {
        return 0;
    };
    wrapper.segments = transaction
        .segments
        .iter()
        .map(|&(begin, end, style)| OxyOSPreeditSegment {
            begin,
            end,
            style: style as u8,
        })
        .collect();
    *out = OxyOSTextTransaction {
        surface: transaction.surface,
        delete_before: transaction.delete_before,
        delete_after: transaction.delete_after,
        cursor_begin: transaction.cursor.0,
        cursor_end: transaction.cursor.1,
        commit_serial: transaction.commit_serial,
        serial: transaction.serial,
        commit: transaction.commit.as_ptr().cast(),
        commit_length: transaction.commit.len(),
        preedit: transaction.preedit.as_ptr().cast(),
        preedit_length: transaction.preedit.len(),
        segments: wrapper.segments.as_ptr(),
        segment_count: wrapper.segments.len(),
    };
    1
}
