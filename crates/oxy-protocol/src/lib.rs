use std::io::{self, Read, Write};

pub const VERSION: u32 = 1;
pub const MAX_PACKET_BYTES: usize = 64;
pub const MAX_DIMENSION: u32 = 16_384;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent {
    pub width: u32,
    pub height: u32,
}

impl Extent {
    pub fn is_valid(self) -> bool {
        (1..=MAX_DIMENSION).contains(&self.width) && (1..=MAX_DIMENSION).contains(&self.height)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub sequence: u64,
    pub buffer: u64,
    pub configure: u64,
    pub extent: Extent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
    Hello { version: u32 },
    Welcome { version: u32 },
    Configure { serial: u64, extent: Extent },
    Submit(Frame),
    Presented { sequence: u64, timestamp_ns: u64 },
    Goodbye,
}

pub fn negotiate(version: u32) -> io::Result<()> {
    if version == VERSION {
        Ok(())
    } else {
        Err(invalid("unsupported protocol version"))
    }
}

fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

pub fn write_message(writer: &mut impl Write, message: Message) -> io::Result<()> {
    let mut body = Vec::with_capacity(MAX_PACKET_BYTES);
    match message {
        Message::Hello { version } | Message::Welcome { version } => {
            body.push(if matches!(message, Message::Hello { .. }) {
                1
            } else {
                2
            });
            body.extend(version.to_le_bytes());
        }
        Message::Configure { serial, extent } => {
            if serial == 0 || !extent.is_valid() {
                return Err(invalid("invalid configure"));
            }
            body.push(3);
            body.extend(serial.to_le_bytes());
            body.extend(extent.width.to_le_bytes());
            body.extend(extent.height.to_le_bytes());
        }
        Message::Submit(frame) => {
            validate_frame(frame)?;
            body.push(4);
            body.extend(frame.sequence.to_le_bytes());
            body.extend(frame.buffer.to_le_bytes());
            body.extend(frame.configure.to_le_bytes());
            body.extend(frame.extent.width.to_le_bytes());
            body.extend(frame.extent.height.to_le_bytes());
        }
        Message::Presented {
            sequence,
            timestamp_ns,
        } => {
            if sequence == 0 {
                return Err(invalid("invalid presentation sequence"));
            }
            body.push(5);
            body.extend(sequence.to_le_bytes());
            body.extend(timestamp_ns.to_le_bytes());
        }
        Message::Goodbye => body.push(6),
    }
    writer.write_all(&(body.len() as u32).to_le_bytes())?;
    writer.write_all(&body)?;
    writer.flush()
}

fn validate_frame(frame: Frame) -> io::Result<()> {
    if frame.sequence == 0 || frame.buffer == 0 || frame.configure == 0 || !frame.extent.is_valid()
    {
        Err(invalid("invalid frame metadata"))
    } else {
        Ok(())
    }
}

pub fn read_message(reader: &mut impl Read) -> io::Result<Message> {
    let mut header = [0; 4];
    reader.read_exact(&mut header)?;
    let length = u32::from_le_bytes(header) as usize;
    if !(1..=MAX_PACKET_BYTES).contains(&length) {
        return Err(invalid("packet length out of bounds"));
    }
    let mut body = [0; MAX_PACKET_BYTES];
    reader.read_exact(&mut body[..length])?;
    let body = &body[..length];
    let u32_at = |offset: usize| {
        u32::from_le_bytes(
            body[offset..offset + 4]
                .try_into()
                .expect("checked message length"),
        )
    };
    let u64_at = |offset: usize| {
        u64::from_le_bytes(
            body[offset..offset + 8]
                .try_into()
                .expect("checked message length"),
        )
    };
    let message = match (body[0], length) {
        (1, 5) => Message::Hello { version: u32_at(1) },
        (2, 5) => Message::Welcome { version: u32_at(1) },
        (3, 17) => Message::Configure {
            serial: u64_at(1),
            extent: Extent {
                width: u32_at(9),
                height: u32_at(13),
            },
        },
        (4, 33) => Message::Submit(Frame {
            sequence: u64_at(1),
            buffer: u64_at(9),
            configure: u64_at(17),
            extent: Extent {
                width: u32_at(25),
                height: u32_at(29),
            },
        }),
        (5, 17) => Message::Presented {
            sequence: u64_at(1),
            timestamp_ns: u64_at(9),
        },
        (6, 1) => Message::Goodbye,
        _ => return Err(invalid("unknown opcode or invalid message length")),
    };
    match message {
        Message::Configure { serial, extent } if serial == 0 || !extent.is_valid() => {
            Err(invalid("invalid configure"))
        }
        Message::Submit(frame) => {
            validate_frame(frame)?;
            Ok(message)
        }
        Message::Presented { sequence: 0, .. } => Err(invalid("invalid presentation sequence")),
        _ => Ok(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn messages_round_trip_without_consuming_the_next_packet() {
        let extent = Extent {
            width: 800,
            height: 600,
        };
        let messages = [
            Message::Hello { version: VERSION },
            Message::Welcome { version: VERSION },
            Message::Configure { serial: 1, extent },
            Message::Submit(Frame {
                sequence: 1,
                buffer: 2,
                configure: 1,
                extent,
            }),
            Message::Presented {
                sequence: 1,
                timestamp_ns: 12,
            },
            Message::Goodbye,
        ];
        let mut bytes = Vec::new();
        for message in messages {
            write_message(&mut bytes, message).unwrap();
        }
        let mut cursor = Cursor::new(bytes);
        for message in messages {
            assert_eq!(read_message(&mut cursor).unwrap(), message);
        }
        assert_eq!(cursor.position(), cursor.get_ref().len() as u64);
    }

    #[test]
    fn rejects_unbounded_lengths_before_reading_the_payload() {
        for length in [0_u32, 65, u32::MAX] {
            assert_eq!(
                read_message(&mut Cursor::new(length.to_le_bytes()))
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidData
            );
        }
    }

    #[test]
    fn rejects_unknown_truncated_and_invalid_packets() {
        for body in [vec![99], vec![1, 1], vec![6, 0], vec![3; 17], vec![4; 33]] {
            let mut packet = (body.len() as u32).to_le_bytes().to_vec();
            packet.extend(body);
            assert!(read_message(&mut Cursor::new(packet)).is_err());
        }
        assert_eq!(
            read_message(&mut Cursor::new([5, 0, 0, 0, 1]))
                .unwrap_err()
                .kind(),
            io::ErrorKind::UnexpectedEof
        );
        assert!(negotiate(VERSION + 1).is_err());
    }

    #[test]
    fn reads_fragmented_streams() {
        struct Fragmented(Cursor<Vec<u8>>);
        impl Read for Fragmented {
            fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                let count = bytes.len().min(1);
                self.0.read(&mut bytes[..count])
            }
        }
        let mut bytes = Vec::new();
        write_message(&mut bytes, Message::Hello { version: VERSION }).unwrap();
        assert_eq!(
            read_message(&mut Fragmented(Cursor::new(bytes))).unwrap(),
            Message::Hello { version: VERSION }
        );
    }
}
