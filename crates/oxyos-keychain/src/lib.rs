//! The keychain's core (OxyOS ADR 0050, step 1): the Oxy Keychain's formats (Oxy ADR 0063 D8),
//! byte for byte as the Oxy SDK writes them, and the machine's own sealed copy of it.
//!
//! - [`seal`]: age's X25519 recipient, the one way a key reaches another key.
//! - [`keys`]: a member device's Ed25519 and X25519 keys; [`recovery`]: the recovery key, its
//!   text and the keys derived from it.
//! - [`keyring`]: every generation of the vault key; [`wrap`]: the keyring sealed to one
//!   recipient and signed, opened only when its signer is in the verified roster.
//! - [`roster`]: the append-only signed membership log and its verifier, which fails closed.
//! - [`enroll`]: the commitment and the six-digit comparison code of adding a device.
//! - [`item`]: the items (logins, passkeys, codes, notes, Wi-Fi), padded, sealed and rolled
//!   forward only.
//! - [`password`]: the strong password generator; [`totp`]: RFC 6238 codes from `otpauth://`.
//! - [`keybag`] and [`store`]: the machine's state, sealed under a key the account's password
//!   opens (Argon2id), in files only the service can read.
//!
//! **Secrets.** Keys, passwords, codes, items and recovery keys are overwritten when dropped
//! ([`zeroize`]) and never appear in `Debug`, `Display` or an error. Long-lived keys are boxed so
//! that moving their holder leaves no copy behind. Locking them against swap (`mlock`) is the
//! service's to do when it holds them (step 2); this crate does not pretend to.
//!
//! **No hand-made cryptography.** Every primitive is RustCrypto's or dalek's; JCS (RFC 8785) is
//! `serde_json_canonicalizer`'s, and JSON that holds secrets is written in its canonical form
//! directly into buffers that are overwritten (checked against the canonicalizer in tests).

pub mod enroll;
mod error;
pub mod item;
pub mod keybag;
pub mod keyring;
pub mod keys;
pub mod password;
pub mod recovery;
pub mod roster;
pub mod seal;
mod secret;
pub mod store;
pub mod totp;
pub mod wire;
pub mod wrap;

pub use error::{Error, Result};
pub use secret::SecretString;
pub use wire::{ItemId, MemberId, VaultId};

#[cfg(test)]
mod tests;
