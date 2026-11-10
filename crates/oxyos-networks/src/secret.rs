//! A password on its way to the daemon (docs/adr/0038-networks.mdx, decision 5): never printed,
//! and its bytes cleared when it goes. The type is the one secret type of OxyOS (`oxyos-secret`);
//! what is a WPA passphrase is this crate's.

pub use oxyos_secret::{clear, Secret};

/// Whether `password` is a WPA passphrase as NetworkManager accepts one, and as the shell's sheet
/// counts it: 8 to 63 bytes of UTF-8, or 64 hexadecimal digits (the key itself).
pub fn valid_wpa(password: &Secret) -> bool {
    let length = password.len();
    (8..=63).contains(&length)
        || (length == 64 && password.as_bytes().iter().all(|b| b.is_ascii_hexdigit()))
}
