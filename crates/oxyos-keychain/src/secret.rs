//! Text that is a secret (a password, a username, a title, a URI holding a seed) is
//! `oxyos-secret`'s [`Secret`]: overwritten when dropped, never printed. And JSON written into
//! buffers that are overwritten.

use serde::Serialize;
use std::io;
use zeroize::Zeroizing;

/// The one secret type of OxyOS (`oxyos-secret`): overwritten when dropped, never printed.
pub use oxyos_secret::Secret;

/// Counts what would be written, so that a secret's buffer is allocated once at its size and
/// never grows (growing would leave a copy in the memory it left).
struct Count(usize);

impl io::Write for Count {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// `value` as JSON, in a buffer overwritten when dropped. Serde writes a struct's fields in their
/// declared order: types written here declare them in RFC 8785's order (sorted by UTF-16 code
/// units, which for their ASCII names is byte order), hold only strings, booleans, integers
/// below 2⁵³ and maps with ASCII keys, so what is written is their JCS form (tested).
pub(crate) fn json(value: &impl Serialize) -> crate::Result<Zeroizing<Vec<u8>>> {
    let mut count = Count(0);
    serde_json::to_writer(&mut count, value).map_err(|_| crate::Error::Malformed("JSON"))?;
    let mut buffer = Zeroizing::new(Vec::with_capacity(count.0));
    serde_json::to_writer(&mut *buffer, value).map_err(|_| crate::Error::Malformed("JSON"))?;
    debug_assert_eq!(buffer.len(), count.0);
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_is_sized_once() {
        let buffer = json(&serde_json::json!({ "a": "b", "c": [1, 2] })).unwrap();
        assert_eq!(buffer.len(), buffer.capacity());
    }
}
