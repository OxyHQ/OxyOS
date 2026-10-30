//! Installing, updating and removing OxyOS applications (docs/adr/0030-installing-applications.mdx,
//! decisions 2 to 6, 10 and 11): the logic of `oxyos-installd`, testable without root.
//!
//! - [`tuf`]: the store's metadata (TUF 1.0's top-level roles) and the client that keeps it.
//! - [`fetch`]: what reads the repository; the fetcher, a service of its own with the network
//!   and nothing else, written into descriptors the install service hands it.
//! - [`compile`]: the runtime's `hermesc`, confined, compiling a package's JavaScript.
//! - [`install`]: releases verified (the store's statement, the developer's signature and key
//!   lineage, the manifest, the runtime), versions assembled and switched, rollback, holds,
//!   consent, removal and collection.
//! - [`state`]: what the service keeps on disk, each file written whole.
//! - [`ipc`] and [`socket`]: the service's socket.
//! - [`accounts`]: the accounts a peer's uid names.

pub mod accounts;
pub mod compile;
pub mod fetch;
pub mod install;
pub mod ipc;
pub mod socket;
pub mod state;
pub mod time;
pub mod tuf;

#[cfg(test)]
mod tests;
