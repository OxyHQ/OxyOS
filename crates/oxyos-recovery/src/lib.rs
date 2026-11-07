//! OxyOS on the disk, and getting it back ([ADR 0054](../../docs/adr/0054-recovery-and-erasing.mdx),
//! which builds on [ADR 0023](../../docs/adr/0023-installing-oxyos.mdx)): writing the tested image
//! onto a disk, the recovery that boots from its own partition, erasing all content and settings,
//! and counting boots so that a system that no longer starts ends up in recovery by itself.
//!
//! - [`layout`]: the disk an install writes (the EFI system partition, OxyOS Recovery, the root)
//!   and systemd-repart's definitions of it.
//! - [`disks`]: the machine's disks as lsblk sees them, the ones an install may take, and the
//!   OxyOS layout found on one.
//! - [`entries`]: the boot loader's entries and their boot counting (systemd's automatic boot
//!   assessment): giving the system its tries back after a complete boot, condemning it while an
//!   erase is pending.
//! - [`request`]: the erase request the running system leaves for the recovery, and when the
//!   recovery may carry it out.
//! - [`install`]: installing from the live medium, and erasing and reinstalling from the recovery.
//! - [`check`]: checking and repairing the disk's file systems.
//! - [`network`]: what the recovery needs of NetworkManager to get online.
//! - [`host`]: the programs every step runs (systemd-repart, cryptsetup, mount, chroot), through
//!   one seam the tests replace.
//!
//! The rules (which partition is which, which entry may boot, whether a request is this disk's,
//! the steps of an install in their order) are pure and tested without root; the programs they run
//! are Debian's and systemd's own, run by [`host::System`]. What only a machine can show (a boot
//! through shim and systemd-boot, a disk erased and the system set up again) is tested in QEMU by
//! `tools/image/acceptance --recovery`.

pub mod check;
pub mod disks;
pub mod entries;
pub mod host;
pub mod install;
pub mod layout;
pub mod network;
pub mod request;

#[cfg(test)]
mod tests;
