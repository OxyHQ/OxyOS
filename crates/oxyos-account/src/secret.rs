//! A secret held in memory: a password, a device secret, an access token, a second-factor code or
//! challenge. Overwritten when dropped, never printed.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use zeroize::Zeroizing;

pub struct Secret(Zeroizing<String>);

impl Secret {
    pub fn new(text: impl Into<String>) -> Self {
        Self(Zeroizing::new(text.into()))
    }

    /// The secret itself, for the one place that hands it on (a request body, `chpasswd`).
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(…)")
    }
}

impl PartialEq for Secret {
    /// Equal secrets compare equal; for tests and for checking an echo, never for a password.
    fn eq(&self, other: &Self) -> bool {
        self.0.as_bytes() == other.0.as_bytes()
    }
}

impl Eq for Secret {}

impl Serialize for Secret {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::new)
    }
}

/// JSON that may hold secrets: written into a buffer that is overwritten when dropped. The buffer
/// is sized beforehand so that growing it leaves no copy behind.
pub(crate) fn json(value: &impl Serialize) -> Result<Zeroizing<Vec<u8>>, serde_json::Error> {
    let mut buffer = Zeroizing::new(Vec::with_capacity(16 * 1024));
    serde_json::to_writer(&mut *buffer, value)?;
    Ok(buffer)
}
