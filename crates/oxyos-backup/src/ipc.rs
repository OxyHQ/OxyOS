//! The backup service's socket, `/run/oxyos/backupd/socket` ([ADR 0059](../../docs/adr/0059-backups.mdx),
//! decision 8): `SOCK_SEQPACKET`, one JSON message per packet of at most 64 KiB, the peer's
//! credentials from the kernel (`SO_PEERCRED`), as the account and install services'.
//!
//! - **Who may ask what.** Anyone may read the state, follow it, back up now, skip a backup and
//!   hold scheduled backups back (a session in Low Power Mode). Only an **administrator** (root, or
//!   a member of `sudo`, ADR 0054 decision 8) adds or removes destinations and changes what is
//!   backed up. A person browses and retrieves only what was **in their own home**.
//! - **Every answer follows the message it answers**, and the service never closes a connection
//!   with a message unread. A connection lasts until its peer closes it; a hold ends with it.
//! - **The conversation:**
//!
//!   ```text
//!   → "status"                                  ← {"status": Status}
//!   → "watch"                                   ← {"status": Status} now, then at every change,
//!                                                  unasked, between answers
//!   → {"backUpNow": {}}                         ← "started"
//!   → {"stop": {}}                              ← "stopping"
//!   → {"hold": {"reason": "lowPower"}}          ← "held"        (until released or closed)
//!   → {"release": {}}                           ← "released"
//!   → {"disks": {}}                             ← {"disks": [Disk]}                    (admin)
//!   → {"addDisk": {"uuid", "password"}}         ← {"added": {"id"}}                    (admin)
//!   → {"addShare": {"share", "sharePassword",   ← {"added": {"id"}}                    (admin)
//!                   "password"}}
//!   → {"remove": {"id"}}                        ← "removed"                            (admin)
//!   → {"setExcluded": {"paths"}}                ← "set"                                (admin)
//!   → {"setAutomatic": {"enabled"}}             ← "set"                                (admin)
//!   → {"versions": {"path"}}                    ← {"versions": [Version]}
//!   → {"contents": {"destination", "snapshot", "path"}}   ← {"contents": [Item]}
//!   → {"retrieve": {"destination", "snapshot", "path"}}   ← {"retrieving": {"kind", "size"}}
//!                                                  with a pipe's reading end: the file's bytes,
//!                                                  or a folder as a tar archive; then, once it
//!                                                  was all written, "retrieved"
//!   ← {"refused": {"reason"}}                   for any of them
//!   ```
//!
//!   Refusals' reasons: `notPermitted`, `invalidRequest`, `noDestination`, `notConnected`,
//!   `running`, `notRunning`, `notFound` (not in the backup, or not the asker's), `wrongPassword`,
//!   `alreadyAdded`, `unreachable`, `full`, `failed`.

use crate::disks::Disk;
use crate::share::Share;
use crate::store::Failure;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use zeroize::Zeroizing;

/// Where the service listens.
pub const SOCKET: &str = "/run/oxyos/backupd/socket";

/// What a peer asks.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum Request {
    Status,
    Watch,
    BackUpNow {},
    Stop {},
    Hold {
        reason: HoldReason,
    },
    Release {},
    Disks {},
    #[serde(rename_all = "camelCase")]
    AddDisk {
        uuid: String,
        /// The person's backup password: a key of the repository, which opens it on another
        /// computer, in the recovery and in setup.
        password: Zeroizing<String>,
    },
    #[serde(rename_all = "camelCase")]
    AddShare {
        share: Share,
        share_password: Zeroizing<String>,
        password: Zeroizing<String>,
    },
    Remove {
        id: String,
    },
    SetExcluded {
        paths: Vec<PathBuf>,
    },
    SetAutomatic {
        enabled: bool,
    },
    Versions {
        path: String,
    },
    Contents {
        destination: String,
        snapshot: String,
        path: String,
    },
    Retrieve {
        destination: String,
        snapshot: String,
        path: String,
    },
}

/// Why scheduled backups are held back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HoldReason {
    /// The session is in Low Power Mode (ADR 0057, decision 4).
    LowPower,
}

/// What the service answers.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Answer {
    Status(Status),
    Started,
    Stopping,
    Held,
    Released,
    Disks(Vec<DiskChoice>),
    Added { id: String },
    Removed,
    Set,
    Versions(Vec<Version>),
    Contents(Vec<Item>),
    Retrieving { kind: ItemKind, size: Option<u64> },
    Retrieved,
    Refused { reason: String },
}

impl Answer {
    pub fn refused(reason: &str) -> Self {
        Answer::Refused {
            reason: reason.to_owned(),
        }
    }
}

/// The service's state, as every peer is told it.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub automatic: bool,
    pub destinations: Vec<DestinationStatus>,
    pub excluded: Vec<PathBuf>,
    /// The backup running now.
    pub running: Option<Running>,
    /// The newest backup of any destination.
    pub last_backup: Option<i64>,
    /// When the next scheduled backup is due (none while not automatic, or with no destination).
    pub next_backup: Option<i64>,
    /// Scheduled backups are held back (Low Power Mode).
    pub held: bool,
    /// Whether the peer told may change what is backed up and where.
    pub administrator: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DestinationStatus {
    pub id: String,
    pub name: String,
    /// `disk` or `share`.
    pub kind: &'static str,
    /// A share's `smb://` URL.
    pub url: Option<String>,
    /// Connected (a disk), or worth trying (a share).
    pub available: bool,
    pub last_backup: Option<i64>,
    pub oldest_backup: Option<i64>,
    pub backups: u64,
    pub used_bytes: Option<u64>,
    pub free_bytes: Option<u64>,
    pub last_failure: Option<Failure>,
}

/// A backup running.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Running {
    pub destination: String,
    /// `preparing`, `backingUp`, `cleaningUp` (thinning, removing what is unneeded) or `checking`.
    pub phase: &'static str,
    /// Asked for (Back Up Now) rather than scheduled.
    pub asked: bool,
    pub percent: f64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub files_done: u64,
    pub files_total: u64,
    pub seconds_remaining: Option<u64>,
}

/// A disk an administrator may choose.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskChoice {
    #[serde(flatten)]
    pub disk: Disk,
    /// It is a destination already.
    pub added: bool,
}

/// A version of an item: what a backup held of it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    pub destination: String,
    pub snapshot: String,
    /// When the backup was made.
    pub time: i64,
    pub kind: ItemKind,
    pub size: Option<u64>,
    /// When the item was last changed, as the backup holds it.
    pub modified: Option<i64>,
}

/// An item of a folder, as a backup held it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub name: String,
    pub path: String,
    pub kind: ItemKind,
    pub size: Option<u64>,
    pub modified: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemKind {
    File,
    Folder,
    Link,
    Other,
}

impl ItemKind {
    /// restic's node type.
    pub fn of(kind: &str) -> Self {
        match kind {
            "file" => ItemKind::File,
            "dir" => ItemKind::Folder,
            "symlink" => ItemKind::Link,
            _ => ItemKind::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_read_strictly() {
        assert!(matches!(
            serde_json::from_str::<Request>("\"status\"").unwrap(),
            Request::Status
        ));
        assert!(matches!(
            serde_json::from_str::<Request>(r#"{"backUpNow":{}}"#).unwrap(),
            Request::BackUpNow {}
        ));
        assert!(matches!(
            serde_json::from_str::<Request>(r#"{"hold":{"reason":"lowPower"}}"#).unwrap(),
            Request::Hold {
                reason: HoldReason::LowPower
            }
        ));
        assert!(serde_json::from_str::<Request>(r#"{"backUpNow":{"x":1}}"#).is_err());
        assert!(serde_json::from_str::<Request>(r#"{"erase":{}}"#).is_err());
        let share: Request = serde_json::from_str(
            r#"{"addShare":{"share":{"host":"nas","share":"B","user":"u"},"sharePassword":"s","password":"p"}}"#,
        )
        .unwrap();
        assert!(matches!(share, Request::AddShare { .. }));
    }

    #[test]
    fn answers_are_written_as_the_session_reads_them() {
        assert_eq!(
            serde_json::to_string(&Answer::Started).unwrap(),
            "\"started\""
        );
        assert_eq!(
            serde_json::to_string(&Answer::refused("notPermitted")).unwrap(),
            r#"{"refused":{"reason":"notPermitted"}}"#
        );
        assert_eq!(
            serde_json::to_string(&Answer::Added { id: "a".into() }).unwrap(),
            r#"{"added":{"id":"a"}}"#
        );
    }
}
