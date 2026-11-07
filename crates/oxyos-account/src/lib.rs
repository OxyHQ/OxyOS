//! Oxy accounts on the machine ([ADR 0020](../../docs/adr/0020-oxy-account-login.mdx)): what
//! `oxyos-accountd`, the system service that holds every enrolled account's device credential,
//! does to set one up.
//!
//! - [`api`]: the calls to Oxy's API it makes, with the SDK's contracts, over verified TLS.
//! - [`store`]: the enrolled accounts and their credentials, in the service's own directory.
//! - [`users`]: each account's Unix user, home and verifier, made with the system's tools.
//! - [`setup`]: setting an account up, online only, undone if it fails halfway.
//! - [`ipc`]: the service's own socket (setup, the accounts, the login program's check), and who
//!   may use it.
//! - [`platform`]: the machine's Oxy session once accounts are set up: the online checks,
//!   revocation, removing an account, applications' tokens and the snapshot they see.
//! - [`session`]: the session socket, where the session registers what it launched and its
//!   applications ask for the platform's session; and its clients.
//! - [`keychain`]: the accounts' keychains (ADR 0050) on both sockets: AutoFill for the session,
//!   the vault for Settings, the unlock for the login program; with the `keychain` feature, the
//!   service's own (`keychain::service`), over `oxyos-keychain`.
//! - [`erase`]: erasing the machine, Settings' request: an administrator's password, the erase
//!   armed by its unit, the accounts signed out online, the machine restarted.
//! - [`pidfd`]: processes known by pidfd, never by a pid alone.
//!
//! Each of the network, the users and the store is a seam: the tests run every rule without
//! root and without the network.
//!
//! Secrets (the password, the device secret, tokens, codes) are [`Secret`]s: overwritten in memory
//! when dropped and never printed, nor are the buffers this crate writes them into or reads them
//! from. Copies made by the libraries they pass through (the HTTP client's buffers, the JSON
//! parser's scratch space for escaped text) are not this crate's to overwrite.

pub mod api;
pub mod erase;
mod id;
pub mod ipc;
pub mod keychain;
pub mod pidfd;
pub mod platform;
mod secret;
pub mod session;
pub mod setup;
pub mod store;
pub mod users;

pub use id::{AccountId, InvalidAccountId, UserName, MAX_USER_NAME};
pub use secret::Secret;

#[cfg(test)]
mod tests;
