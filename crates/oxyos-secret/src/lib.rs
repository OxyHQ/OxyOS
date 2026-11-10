//! A secret held in memory: a password, a passcode, a device secret, an access token, a
//! verification code, a credential the keychain fills. One type for every OxyOS program, so each
//! holds its secrets the same way:
//!
//! - **Overwritten when it goes.** The text lives in a buffer that [`zeroize`] overwrites, every
//!   byte of its capacity, when the secret is dropped; a clone is a second such buffer.
//! - **Never printed.** `Debug` says `Secret(…)`; there is no `Display`. The text is read only
//!   through [`Secret::expose`] (or [`Secret::as_bytes`] for what takes bytes, PAM), at the one
//!   place that hands it on.
//! - **Text.** A secret is UTF-8: what a person types reaches OxyOS as text, and every secret the
//!   services keep is a string. Bytes off a wire become a secret through [`Secret::from_utf8`],
//!   which overwrites what it refuses.
//!
//! Comparing two secrets (`==`) is for tests and for checking an echo, never for checking a
//! password: it is not constant-time.

use std::fmt;
use zeroize::{Zeroize, Zeroizing};

#[derive(Clone, Default)]
pub struct Secret(Zeroizing<String>);

/// Bytes that are not UTF-8, refused by [`Secret::from_utf8`]; they were overwritten.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotText;

impl fmt::Display for NotText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a secret is UTF-8 text")
    }
}

impl std::error::Error for NotText {}

impl Secret {
    /// The secret holding `text`, which moves into it (a `String` is not copied).
    pub fn new(text: impl Into<String>) -> Self {
        Self(Zeroizing::new(text.into()))
    }

    /// The secret holding `bytes`, which move into it, if they are UTF-8; otherwise they are
    /// overwritten and refused.
    pub fn from_utf8(bytes: Vec<u8>) -> Result<Self, NotText> {
        match String::from_utf8(bytes) {
            Ok(text) => Ok(Self::new(text)),
            Err(error) => {
                error.into_bytes().zeroize();
                Err(NotText)
            }
        }
    }

    /// The secret itself, for the one place that hands it on (a request body, `chpasswd`, a
    /// field filled).
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// The secret's bytes, for what takes bytes (PAM, a packet).
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// The secret's buffer, still overwritten when dropped (moved, not copied).
    pub fn into_bytes(mut self) -> Zeroizing<Vec<u8>> {
        Zeroizing::new(std::mem::take(&mut *self.0).into_bytes())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Its length in bytes.
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl From<Zeroizing<String>> for Secret {
    fn from(text: Zeroizing<String>) -> Self {
        Self(text)
    }
}

impl Zeroize for Secret {
    fn zeroize(&mut self) {
        self.0.zeroize();
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

#[cfg(feature = "serde")]
impl serde::Serialize for Secret {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Secret {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::new)
    }
}

/// Overwrites `text`, every byte of its capacity, and empties it: for a secret that arrived in a
/// plain `String` (a request's field) before it becomes a [`Secret`] or goes.
pub fn clear(text: &mut String) {
    text.zeroize();
}

#[cfg(test)]
mod tests;
