//! Backups, as Time Machine makes them ([ADR 0059](../../docs/adr/0059-backups.mdx)): every hour
//! to a disk or a share, kept by the hour, the day and the week, encrypted, browsed back in time
//! and restored, through restic.
//!
//! - [`engine`]: restic, run as a program, its JSON read;
//! - [`share`]: a share over SMB, through rclone, which restic starts;
//! - [`disks`]: the disks that can hold backups, followed through udev, mounted while used;
//! - [`store`]: the service's directory: destinations, records, keys;
//! - [`policy`]: the schedule, the thinning and what is read;
//! - [`machine`]: the machine's manifest, for a restore;
//! - [`ipc`]: the service's socket and its vocabulary;
//! - [`service`]: the service: its schedule, its backups and its conversations.

pub mod disks;
pub mod engine;
pub mod ipc;
pub mod machine;
pub mod policy;
pub mod service;
pub mod share;
pub mod store;

#[cfg(test)]
mod tests;
