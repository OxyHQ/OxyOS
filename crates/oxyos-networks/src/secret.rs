//! A password on its way to the daemon (docs/adr/0038-networks.mdx, decision 5): never printed,
//! and its bytes cleared when it goes.

use std::fmt;

/// A Wi-Fi password the person typed into the shell's sheet. `Debug` says nothing of it; dropped,
/// its buffer is overwritten before it is freed.
pub struct Secret(String);

impl Secret {
    pub fn new(text: String) -> Self {
        Self(text)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    /// A WPA passphrase as NetworkManager accepts one, and as the shell's sheet counts it: 8 to
    /// 63 bytes of UTF-8, or 64 hexadecimal digits (the key itself).
    pub fn valid_wpa(&self) -> bool {
        let length = self.0.len();
        (8..=63).contains(&length)
            || (length == 64 && self.0.bytes().all(|b| b.is_ascii_hexdigit()))
    }
}

/// Overwrites `text`'s bytes and empties it.
pub fn clear(text: &mut String) {
    let mut bytes = std::mem::take(text).into_bytes();
    bytes.iter_mut().for_each(|byte| *byte = 0);
    std::hint::black_box(&bytes);
}

impl Drop for Secret {
    fn drop(&mut self) {
        clear(&mut self.0);
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret(…)")
    }
}
