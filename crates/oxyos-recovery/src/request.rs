//! The erase request ([ADR 0054](../../../docs/adr/0054-recovery-and-erasing.mdx), decision 4): what
//! the running system leaves on the EFI system partition for the recovery to carry out at the next
//! boot, and when the recovery may.
//!
//! The request names the disk (its partition table's UUID) and the root it was made for (the root
//! partition's UUID). The running system writes it `armed`; the recovery rewrites it `erasing`
//! before it destroys anything, and removes it once the system is set up again. So:
//!
//! - an `armed` request is carried out only on the disk and the root it names: one left behind on a
//!   disk installed again since, or copied to another, erases nothing;
//! - an `erasing` request is carried out on its disk whatever the root is now, since that root may
//!   already be half erased: a power cut in the middle resumes the erase, never boots what is left;
//! - a request that cannot be read is carried out on no disk.
//!
//! It is written whole: to a new file beside it, flushed, renamed over, the directory flushed.
//!
//! A request may also say which backup the new root is restored from (*Restore from Backup*,
//! [ADR 0059](../../../docs/adr/0059-backups.mdx), decision 7): where it is and which computer's
//! and backup it is, never a password (the EFI system partition is read by anyone holding the
//! disk). A power cut during that restore resumes it as an erase does, the recovery asking for the
//! backup password again.

use crate::disks::Layout;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::Path;

/// The request's place on the EFI system partition, from its root.
pub const PATH: &str = "oxyos-recovery/erase.json";

const VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    /// Left by the running system: nothing is erased yet.
    Armed,
    /// The recovery has begun: the root may already be erased.
    Erasing,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EraseRequest {
    pub version: u32,
    pub state: State,
    /// The disk's partition table UUID.
    pub disk: String,
    /// The root partition's UUID when the request was made.
    pub root: String,
    /// Whether the root is made again encrypted (it was): an erase never turns encryption off.
    pub encrypted: bool,
    /// When it was made, in seconds since the epoch: for the log.
    pub requested_at: u64,
    /// The backup the new root is restored from, if it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore: Option<Restore>,
}

/// The backup a new root is restored from, named without its passwords.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Restore {
    /// Where the backup is, as the backups' library names it (a disk by its file system's UUID,
    /// or a share by its computer, name and user): kept as that library wrote it.
    pub source: serde_json::Value,
    /// The computer's repository in `OxyOS Backups`.
    pub repository: String,
    /// The backup, restic's snapshot.
    pub snapshot: String,
    /// The computer's name and the backup's time, to tell the person what is being restored.
    pub computer: String,
    pub time: i64,
}

impl EraseRequest {
    /// An armed request for the root of `layout`.
    pub fn arm(layout: &Layout, requested_at: u64) -> Result<Self, String> {
        Ok(Self {
            version: VERSION,
            state: State::Armed,
            disk: layout
                .disk
                .ptuuid
                .clone()
                .ok_or("the disk has no partition table UUID")?,
            root: layout
                .root
                .partuuid
                .clone()
                .ok_or("the root partition has no UUID")?,
            encrypted: layout.root.encrypted(),
            requested_at,
            restore: None,
        })
    }

    /// Whether the recovery may carry it out on `layout`.
    pub fn applies_to(&self, layout: &Layout) -> Verdict {
        let same = |a: &str, b: Option<&String>| b.is_some_and(|b| b.eq_ignore_ascii_case(a));
        if !same(&self.disk, layout.disk.ptuuid.as_ref()) {
            return Verdict::OtherDisk;
        }
        match self.state {
            State::Erasing => Verdict::Proceed,
            State::Armed if same(&self.root, layout.root.partuuid.as_ref()) => Verdict::Proceed,
            State::Armed => Verdict::OtherRoot,
        }
    }
}

/// Whether a request is this disk's to carry out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Proceed,
    /// It names another disk.
    OtherDisk,
    /// It was armed for a root that is no longer there (the disk was installed again since).
    OtherRoot,
}

/// What reading the request found.
#[derive(Debug)]
pub enum Found {
    None,
    Request(EraseRequest),
    /// There is a file, and it is no request of a version this reads: it erases nothing.
    Unreadable(String),
}

/// The request on the EFI system partition mounted at `esp`.
pub fn read(esp: &Path) -> io::Result<Found> {
    let bytes = match fs::read(esp.join(PATH)) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Found::None),
        Err(e) => return Err(e),
    };
    Ok(match serde_json::from_slice::<EraseRequest>(&bytes) {
        Ok(request) if request.version == VERSION => Found::Request(request),
        Ok(request) => Found::Unreadable(format!("version {}", request.version)),
        Err(e) => Found::Unreadable(e.to_string()),
    })
}

/// Writes `request` whole on the EFI system partition mounted at `esp`.
pub fn write(esp: &Path, request: &EraseRequest) -> io::Result<()> {
    let path = esp.join(PATH);
    let directory = path.parent().expect("the request is in a directory");
    fs::create_dir_all(directory)?;
    let new = path.with_extension("json.new");
    {
        let mut file = fs::File::create(&new)?;
        file.write_all(&serde_json::to_vec_pretty(request).map_err(io::Error::other)?)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }
    fs::rename(&new, &path)?;
    fs::File::open(directory)?.sync_all()
}

/// Removes the request: the erase is done, or it was not this disk's.
pub fn remove(esp: &Path) -> io::Result<()> {
    let path = esp.join(PATH);
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    }
    fs::File::open(path.parent().expect("the request is in a directory"))?.sync_all()
}
