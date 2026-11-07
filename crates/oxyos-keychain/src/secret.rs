//! Text that is a secret (a password, a username, a title, a URI holding a seed): overwritten
//! when dropped, never printed; and JSON written into buffers that are overwritten.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::io;
use zeroize::{Zeroize, Zeroizing};

#[derive(Clone, Default, Zeroize)]
pub struct SecretString(Zeroizing<String>);

impl SecretString {
    pub fn new(text: impl Into<String>) -> Self {
        Self(Zeroizing::new(text.into()))
    }

    /// The text itself, for the one place that hands it on.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl From<Zeroizing<String>> for SecretString {
    fn from(text: Zeroizing<String>) -> Self {
        Self(text)
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretString(…)")
    }
}

impl PartialEq for SecretString {
    /// For tests and for comparing an item with itself; not a password check.
    fn eq(&self, other: &Self) -> bool {
        self.0.as_bytes() == other.0.as_bytes()
    }
}

impl Eq for SecretString {}

impl Serialize for SecretString {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::new)
    }
}

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
    fn debug_hides_the_text() {
        let secret = SecretString::new("hunter2-correct-horse");
        let shown = format!("{secret:?} {:?}", Some(&secret));
        assert!(!shown.contains("hunter2"), "{shown}");
    }

    #[test]
    fn json_is_sized_once() {
        let buffer = json(&serde_json::json!({ "a": "b", "c": [1, 2] })).unwrap();
        assert_eq!(buffer.len(), buffer.capacity());
    }
}
