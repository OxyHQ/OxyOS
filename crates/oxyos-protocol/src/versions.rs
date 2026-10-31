//! What each protocol version is on the wire (ADR 0033). A version only adds: messages with new
//! opcodes, and fields at the end of existing ones. So an event is encoded for an older version by
//! leaving out the fields that version lacks, and decoded from one by giving them the value that
//! means what the older version meant; a message an older version does not have is never sent to
//! it, and refused from it.

use crate::schema::{Event, Request};
use crate::{invalid, MAX_PACKET_BYTES};
use std::io::{self, Read, Write};

/// What version 3 added to the end of events version 2 had: the opcode, how many bytes, and what
/// they are when a version 2 compositor sends the event (no device time; a scroll of pixels from a
/// continuous source, never a stop).
const GROWN_IN_3: &[(u8, &[u8])] = &[
    (4, &[0; 8]),
    (5, &[0; 8]),
    (7, &[0; 8]),
    (11, &[0, 0, 0, 0, 0, 0, 0, 0, 3, 0]),
    (36, &[0; 8]),
    (43, &[0; 8]),
    (150, &[0; 8]),
];

/// The opcodes each version added, by direction (a request and an event may share an opcode):
/// version 3, GPU buffers (ADR 0031), both ways; version 4, a touchpad's resting fingers (Hold),
/// sent unasked to the surface under the pointer; version 5, the session's keyboard layout and
/// the outputs, told to every client, and a client's pointer images (ADR 0029).
type Added = &'static [(u16, std::ops::RangeInclusive<u8>)];
const EVENTS_ADDED: Added = &[(3, 170..=174), (4, 58..=58), (5, 180..=180), (5, 200..=203)];
const REQUESTS_ADDED: Added = &[(3, 170..=174), (5, 190..=190)];

fn since(added: Added, opcode: u8) -> u16 {
    added
        .iter()
        .find(|(_, opcodes)| opcodes.contains(&opcode))
        .map_or(2, |(version, _)| *version)
}

/// The version that introduced an event: it is never sent to a connection of an older one.
pub fn event_since(event: &Event) -> u16 {
    since(EVENTS_ADDED, event.opcode())
}

/// The version that introduced a request: a compositor refuses it from a connection of an older
/// one.
pub fn request_since(request: &Request) -> u16 {
    since(REQUESTS_ADDED, request.opcode())
}

fn grown(opcode: u8, version: u16) -> &'static [u8] {
    if version >= 3 {
        return &[];
    }
    GROWN_IN_3
        .iter()
        .find(|(grown, _)| *grown == opcode)
        .map_or(&[], |(_, fields)| fields)
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
    body.extend_from_slice(grown(body[0], version));
    let event = Event::decode(&body)?;
    if event_since(&event) > version {
        return Err(invalid("an event the connection's version does not have"));
    }
    Ok(event)
}
