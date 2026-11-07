//! What each protocol version is on the wire (ADR 0033). A version only adds: messages with new
//! opcodes, and fields at the end of existing ones. So an event is encoded for an older version by
//! leaving out the fields that version lacks, and decoded from one by giving them the value that
//! means what the older version meant; a message an older version does not have is never sent to
//! it, and refused from it.

use crate::schema::{Event, Request};
use crate::wire::{
    AccessibleField, AccessibleRole, ApplicationListKind, ShortcutIntent, SystemShortcut,
};
use crate::{invalid, MAX_PACKET_BYTES};
use std::io::{self, Read, Write};

/// What versions added to the end of events an older version had, in the order they added it:
/// the version, the opcode, and what the added bytes are when a compositor of an older version
/// sends the event. Version 3: each input event's device time (none) and a scroll's source and
/// stop (pixels from a continuous source, never a stop). Version 8: a pointer event's modifiers
/// (none held) and whether a window's process is responding (it is). Version 11: whether a text
/// field's caret blinks (it does). Version 19: whether a press made its window the active one (it
/// did not: an older compositor delivers every press as one the view under it takes, ADR 0016).
/// Version 25: what a system shortcut asks of what the shell shows (a key's toggle,
/// docs/adr/0046-interactive-gestures.mdx). Version 26: how many media types of its files a drag
/// entering a target carries (none: an older compositor never tells them, ADR 0035).
const GROWN: &[(u16, u8, &[u8])] = &[
    (3, 4, &[0; 8]),
    (3, 5, &[0; 8]),
    (3, 7, &[0; 8]),
    (3, 11, &[0, 0, 0, 0, 0, 0, 0, 0, 3, 0]),
    (3, 36, &[0; 8]),
    (3, 43, &[0; 8]),
    (3, 150, &[0; 8]),
    (8, 4, &[0]),
    (8, 37, &[1]),
    (11, 53, &[1]),
    (19, 4, &[0]),
    (25, 100, &[0]),
    (26, 240, &[0]),
];

/// The opcodes each version added, by direction (a request and an event may share an opcode):
/// version 3, GPU buffers (ADR 0031), both ways; version 4, a touchpad's resting fingers (Hold),
/// sent unasked to the surface under the pointer; version 5, the session's keyboard layout and
/// the outputs, told to every client, and a client's pointer images (ADR 0029); version 6,
/// screenshots (ADR 0024, decision 10), both ways, `CaptureMode` sent unasked on Super+Shift+4
/// and 5; version 7, file choosers (ADR 0029, decision 8), both ways, so the client library never
/// sends a request a compositor of an older version would close the connection for; version 8,
/// the dock's application menu (ADR 0034), both ways, `Ping` sent unasked; version 9, a popup's
/// parent tracking the pointer during its grab (`TrackPointer`, a menu bar, ADR 0001); version 10,
/// drag and drop (ADR 0035), both ways, a target's events sent unasked; version 13, a node's place
/// in its collection (`AccessiblePosition`, ADR 0039), both ways, sent with every snapshot;
/// version 17, the session's locale (`Locale`, ADR 0040), told to every client; version 18, input
/// methods' candidates drawn by the shell and a key's symbol (ADR 0041), both ways, sent only to
/// an input method or the shell; version 20, where the shell's dock has each window minimize into
/// (`ClearMinimizeTargets`, `PlaceMinimizeTarget`, ADR 0011's genie); version 21, the shell's
/// candidate window resized in place (`ResizeInputCandidates`, ADR 0041); version 22, whether a
/// zoomed or full-screen window covers an output (`OutputCovered`, ADR 0034), told to the shell;
/// version 23, system sounds (`PlaySystemSound`, `NotificationSound`,
/// docs/adr/0044-system-sounds.mdx), asked by any client;
/// version 24, the shell's windows moved into their views (`MoveWindowsToViews`, ADR 0045);
/// version 25, which view a touchpad's swipe drives a shell surface is (`SetGestureOverlay`,
/// docs/adr/0046-interactive-gestures.mdx); version 26, a drag vanishing where it is dropped on
/// nothing, its files' media types, and the shell's files opened with an application (ADR 0035
/// and ADR 0034), both ways, a target's events sent unasked; version 28, each output's wallpaper
/// (`OutputWallpaper`, docs/adr/0048-wallpapers.mdx), told to the shell.
type Added = &'static [(u16, std::ops::RangeInclusive<u8>)];
const EVENTS_ADDED: Added = &[
    (3, 170..=174),
    (4, 58..=58),
    (5, 180..=180),
    (5, 200..=203),
    (6, 210..=212),
    (7, 220..=220),
    (8, 230..=232),
    (10, 240..=250),
    (13, 251..=251),
    (17, 181..=181),
    (18, 59..=64),
    (22, 81..=81),
    (26, 252..=252),
    (28, 82..=82),
];
const REQUESTS_ADDED: Added = &[
    (3, 170..=174),
    (5, 190..=190),
    (6, 210..=215),
    (7, 220..=222),
    (8, 230..=234),
    (9, 235..=235),
    (10, 240..=247),
    (13, 251..=251),
    (18, 63..=67),
    (20, 92..=93),
    (21, 68..=68),
    (23, 236..=237),
    (24, 94..=94),
    (25, 95..=95),
    (26, 248..=250),
];

fn since(added: Added, opcode: u8) -> u16 {
    added
        .iter()
        .find(|(_, opcodes)| opcodes.contains(&opcode))
        .map_or(2, |(version, _)| *version)
}

/// The version that introduced an event: it is never sent to a connection of an older one. An
/// event an older version had, carrying a value that version lacks, is the newer version's too:
/// version 12's system shortcuts to the dock and to the status items (ADR 0039), which a version 11
/// reader would refuse as a value outside its set; version 13's hint of an accessible node
/// (`AccessibleField::Hint`), which a version 12 reader would refuse the same way; version 14's
/// column header role (`AccessibleRole::ColumnHeader`) and version 15's meter
/// (`AccessibleRole::Meter`), likewise; and version 16's shortcut to the Notification Center
/// (ADR 0039); version 23's list of applications whose notifications play no sound
/// (`ApplicationListKind::NotificationSoundsOff`); version 24's shortcuts to Mission Control and
/// App Exposé (ADR 0045); and version 25's shortcut opening or closing what it shows, a swipe's
/// (docs/adr/0046-interactive-gestures.mdx), which a version 24 shell would take for a toggle.
pub fn event_since(event: &Event) -> u16 {
    match event {
        Event::SystemShortcut {
            intent: ShortcutIntent::Open | ShortcutIntent::Close,
            ..
        } => 25,
        Event::SystemShortcut {
            shortcut: SystemShortcut::MissionControl | SystemShortcut::AppExpose,
            ..
        } => 24,
        Event::SystemShortcut {
            shortcut: SystemShortcut::Dock | SystemShortcut::StatusMenus,
            ..
        } => 12,
        Event::SystemShortcut {
            shortcut: SystemShortcut::Notifications,
            ..
        } => 16,
        Event::AccessibleText {
            field: AccessibleField::Hint,
            ..
        } => 13,
        Event::AccessibleNode {
            role: AccessibleRole::ColumnHeader,
            ..
        } => 14,
        Event::AccessibleNode {
            role: AccessibleRole::Meter,
            ..
        } => 15,
        Event::ApplicationList {
            list: ApplicationListKind::NotificationSoundsOff,
            ..
        } => 23,
        _ => since(EVENTS_ADDED, event.opcode()),
    }
}

/// The version that introduced a request: a compositor refuses it from a connection of an older
/// one. A request an older version had, carrying a value that version lacks, is the newer
/// version's: a node's hint (version 13), a column header (version 14) and a meter (version 15),
/// which an older compositor would refuse; and that list (version 23).
pub fn request_since(request: &Request) -> u16 {
    match request {
        Request::AccessibleText {
            field: AccessibleField::Hint,
            ..
        } => 13,
        Request::AccessibleNode {
            role: AccessibleRole::ColumnHeader,
            ..
        } => 14,
        Request::AccessibleNode {
            role: AccessibleRole::Meter,
            ..
        } => 15,
        Request::SetApplicationList {
            list: ApplicationListKind::NotificationSoundsOff,
            ..
        } => 23,
        _ => since(REQUESTS_ADDED, request.opcode()),
    }
}

/// The bytes versions newer than `version` added to the end of the event `opcode`, in order: what
/// a writer for `version` leaves out, and what a reader of `version` puts back.
fn grown(opcode: u8, version: u16) -> Vec<u8> {
    GROWN
        .iter()
        .filter(|(since, grown, _)| *grown == opcode && *since > version)
        .flat_map(|(_, _, fields)| fields.iter().copied())
        .collect()
}

/// Writes `event` as `version` defines it; false, writing nothing, when `version` does not have it.
pub fn write_event_for(writer: &mut impl Write, event: &Event, version: u16) -> io::Result<bool> {
    if event_since(event) > version {
        return Ok(false);
    }
    let mut body = event.encode()?;
    body.truncate(body.len() - grown(event.opcode(), version).len());
    let mut packet = (body.len() as u32).to_le_bytes().to_vec();
    packet.extend_from_slice(&body);
    writer.write_all(&packet)?;
    writer.flush()?;
    Ok(true)
}

/// Reads an event a compositor of `version` wrote, as this version's event.
pub fn read_event_for(reader: &mut impl Read, version: u16) -> io::Result<Event> {
    let mut header = [0; 4];
    reader.read_exact(&mut header)?;
    let length = u32::from_le_bytes(header) as usize;
    if !(1..=MAX_PACKET_BYTES).contains(&length) {
        return Err(invalid("packet length out of bounds"));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    body.extend_from_slice(&grown(body[0], version));
    let event = Event::decode(&body)?;
    if event_since(&event) > version {
        return Err(invalid("an event the connection's version does not have"));
    }
    Ok(event)
}
