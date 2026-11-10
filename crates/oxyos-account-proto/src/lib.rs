//! The accounts service's messages (`oxyos-accountd`, ADR 0020 and ADR 0050): what its two sockets
//! carry, and the clients' side of asking them. The service (`oxyos-account`) and every program
//! that speaks to it (the login screen, the session, applications through `oxyos-client-ffi`) share
//! these types, so a client links the messages alone and never the service: its store, its
//! keychain's cryptography, its PIN verifiers.
//!
//! The modules mirror `oxyos-account`'s, which re-exports each item where it was.

pub mod api;
pub mod erase;
pub mod ipc;
pub mod keychain;
pub mod pin;
pub mod platform;
pub mod session;
pub mod setup;
pub mod store;

/// The one secret type of OxyOS (`oxyos-secret`): overwritten when dropped, never printed.
pub use oxyos_secret::Secret;
