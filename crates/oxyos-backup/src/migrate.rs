//! *Migrate from a Backup* in setup ([ADR 0059](../../docs/adr/0059-backups.mdx), decision 7): the
//! login screen of a computer nobody has set up yet restores another computer's backup into the
//! running system, through `oxyos-backupd --migrate`, a service of its own started for it.
//!
//! - **Not the backup service.** `oxyos-backupd` runs always and may write nothing but its own
//!   state (`ProtectSystem=strict`, `ProtectHome=read-only`); a migration makes users in `/etc`
//!   and writes their homes. So it is another unit, `oxyos-migrate.service`, root with the
//!   privileges a restore needs and no more, started by its socket (`oxyos-migrate.socket`) when
//!   the login screen connects, and gone once nobody has for a minute: those privileges exist only
//!   while a migration is asked for.
//! - **Who may ask, and when**: root and the login program's user (the login screen's), by the
//!   kernel's credentials (`SO_PEERCRED`), and only while no account is enrolled on the computer:
//!   once one is, every request is refused `alreadySetUp`, and the restore itself refuses a root
//!   with an account ([`crate::restore`]).
//! - **One conversation**: the backup password is given once, with the source; the disk stays
//!   mounted, in the service's own namespace, until the conversation ends.
//!
//!   ```text
//!   → {"disks": {}}                               ← {"disks": [Disk]}
//!   → {"backups": {"source", "password",          ← {"backups": {"computers": [Computer],
//!                  "sharePassword"?}}                            "unopened"}}
//!   → {"migrate": {"repository", "snapshot"}}     ← {"migrating": {"step", "percent"}} …
//!                                                 ← {"migrated": {"computer", "accounts"}}
//!   ← {"refused": {"reason"}}                     for any of them
//!   ```
//!
//!   `step` is `accounts`, `files` or `finishing`; `accounts` are `{"user", "name"}`, which the
//!   login screen then lists to sign in again. Refusals' reasons: `notPermitted`, `alreadySetUp`,
//!   `invalidRequest`, `notOpened` (a `migrate` before `backups`), `notConnected`, `noBackup`,
//!   `wrongPassword` (the password opens none of the backups there), `shareRefused`,
//!   `unreachable`, `inUse`, `full`, `invalidBackup`, `clash`, `failed`.

use crate::disks::Disk;
use crate::engine::Stop;
use crate::restore::{Folder, Listing, Refusal, Restorer, Source, Step};
use oxyos_account::store::Store as Accounts;
use oxyos_service_socket::Connection;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use zeroize::Zeroizing;

/// Where the migration service's socket is (`oxyos-migrate.socket`).
pub const SOCKET: &str = "/run/oxyos/migrate/socket";

/// What the login screen asks.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum Request {
    Disks {},
    #[serde(rename_all = "camelCase")]
    Backups {
        source: Source,
        password: Zeroizing<String>,
        #[serde(default)]
        share_password: Option<Zeroizing<String>>,
    },
    Migrate {
        repository: String,
        snapshot: String,
    },
}

/// What the service answers.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Answer {
    Disks(Vec<Disk>),
    Backups(Listing),
    Migrating {
        step: &'static str,
        /// From 0 to 1, the files' share restored.
        percent: f64,
    },
    Migrated {
        computer: String,
        accounts: Vec<Migrated>,
    },
    Refused {
        reason: String,
    },
}

/// An account brought over.
#[derive(Debug, Serialize)]
pub struct Migrated {
    pub user: String,
    pub name: String,
}

impl Answer {
    fn refused(reason: &str) -> Self {
        Answer::Refused {
            reason: reason.to_owned(),
        }
    }
}

/// The migration service.
pub struct Migration {
    pub restorer: Restorer,
    /// The root restored into: `/`, the running system.
    pub target: PathBuf,
    /// The login program's user, which may ask besides root.
    pub login: Option<u32>,
    /// Whether NetworkManager reads the Wi-Fi networks again once they are restored (the
    /// running system's).
    pub reload_networks: bool,
}

impl Migration {
    fn permitted(&self, uid: u32) -> bool {
        uid == 0 || Some(uid) == self.login
    }

    /// Whether an account is enrolled on the target: a migration is setup's, before any.
    fn set_up(&self) -> bool {
        let directory = self.target.join("var/lib/oxyos/accounts");
        if !directory.exists() {
            return false;
        }
        // An account service's directory that cannot be read is no computer to migrate onto.
        Accounts::open(&directory)
            .and_then(|store| store.users())
            .map_or(true, |users| !users.is_empty())
    }

    /// One conversation, to its end.
    pub fn serve(&self, connection: &Connection) -> io::Result<()> {
        let peer = connection.peer()?;
        let mut opened: Option<Folder> = None;
        let mut password = Zeroizing::new(String::new());
        loop {
            let request = match connection.receive::<Request>() {
                Ok(Some(Some(request))) => request,
                Ok(Some(None)) => {
                    connection.send(&Answer::refused("invalidRequest"))?;
                    continue;
                }
                Ok(None) => return Ok(()),
                Err(error) => return Err(error),
            };
            if !self.permitted(peer.uid) {
                connection.send(&Answer::refused("notPermitted"))?;
                continue;
            }
            if self.set_up() {
                connection.send(&Answer::refused("alreadySetUp"))?;
                continue;
            }
            match request {
                Request::Disks {} => connection.send(&Answer::Disks(self.restorer.disks()))?,
                Request::Backups {
                    source,
                    password: given,
                    share_password,
                } => {
                    // The disk before is unmounted first: one source at a time.
                    opened = None;
                    let answer = self
                        .restorer
                        .open(&source, share_password.as_deref().map(String::as_str))
                        .and_then(|folder| {
                            let listing = self.restorer.computers(&folder, &given)?;
                            if listing.computers.is_empty() && listing.unopened > 0 {
                                return Err(Refusal::WrongPassword);
                            }
                            if listing.computers.is_empty() {
                                return Err(Refusal::NoBackup);
                            }
                            opened = Some(folder);
                            Ok(listing)
                        });
                    match answer {
                        Ok(listing) => {
                            password = given;
                            connection.send(&Answer::Backups(listing))?;
                        }
                        Err(refusal) => {
                            eprintln!("oxyos-migrate: the backups: {refusal}");
                            connection.send(&Answer::refused(refusal.reason()))?;
                        }
                    }
                }
                Request::Migrate {
                    repository,
                    snapshot,
                } => {
                    let Some(folder) = &opened else {
                        connection.send(&Answer::refused("notOpened"))?;
                        continue;
                    };
                    let Some(repository) = folder.repository(&repository, &password) else {
                        connection.send(&Answer::refused("invalidRequest"))?;
                        continue;
                    };
                    if !(8..=64).contains(&snapshot.len())
                        || !snapshot.bytes().all(|b| b.is_ascii_hexdigit())
                    {
                        connection.send(&Answer::refused("invalidRequest"))?;
                        continue;
                    }
                    let answer = self.migrate(connection, &repository, &snapshot);
                    connection.send(&answer)?;
                }
            }
        }
    }

    fn migrate(
        &self,
        connection: &Connection,
        repository: &crate::engine::Repository,
        snapshot: &str,
    ) -> Answer {
        let restored =
            self.restorer
                .restore(repository, snapshot, &self.target, &Stop::new(), |step| {
                    let (step, percent) = match step {
                        Step::Accounts => ("accounts", 0.0),
                        Step::Files(progress) => ("files", progress.percent_done.clamp(0.0, 1.0)),
                        Step::Finishing => ("finishing", 1.0),
                    };
                    // A login screen that went away does not stop the restore: it is finished, and
                    // the accounts are there for the next.
                    let _ = connection.send(&Answer::Migrating { step, percent });
                });
        match restored {
            Ok(restored) => {
                eprintln!(
                    "oxyos-migrate: {} account(s) of {} restored",
                    restored.accounts.len(),
                    restored.computer
                );
                if self.reload_networks {
                    reload_networks();
                }
                Answer::Migrated {
                    computer: restored.computer,
                    accounts: restored
                        .accounts
                        .into_iter()
                        .map(|a| Migrated {
                            user: a.user,
                            name: a.name,
                        })
                        .collect(),
                }
            }
            Err(refusal) => {
                eprintln!("oxyos-migrate: the restore: {refusal}");
                Answer::refused(refusal.reason())
            }
        }
    }
}

/// NetworkManager reads its connections again, the restored Wi-Fi networks among them.
fn reload_networks() {
    let reloaded = Command::new("/usr/bin/nmcli")
        .args(["connection", "reload"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if !reloaded.is_ok_and(|s| s.success()) {
        eprintln!("oxyos-migrate: NetworkManager did not read the Wi-Fi networks again");
    }
}

/// The login program's user's uid, from a passwd(5) file.
pub fn login_user(passwd: &Path) -> Option<u32> {
    oxyos_account::users::passwd_entry(passwd, "oxyos-login")
        .ok()
        .flatten()
        .map(|user| user.uid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_read_strictly_and_answers_written_as_the_login_screen_reads_them() {
        let request: Request = serde_json::from_str(
            r#"{"backups":{"source":{"disk":{"uuid":"1234-ABCD"}},"password":"p"}}"#,
        )
        .unwrap();
        assert!(matches!(request, Request::Backups { .. }));
        let share: Request = serde_json::from_str(
            r#"{"backups":{"source":{"share":{"share":{"host":"nas","share":"B","user":"u"}}},"password":"p","sharePassword":"s"}}"#,
        )
        .unwrap();
        assert!(matches!(
            share,
            Request::Backups {
                share_password: Some(_),
                ..
            }
        ));
        assert!(serde_json::from_str::<Request>(r#"{"migrate":{"repository":"a"}}"#).is_err());
        assert!(serde_json::from_str::<Request>(r#"{"erase":{}}"#).is_err());
        assert_eq!(
            serde_json::to_string(&Answer::Migrating {
                step: "files",
                percent: 0.5
            })
            .unwrap(),
            r#"{"migrating":{"step":"files","percent":0.5}}"#
        );
        assert_eq!(
            serde_json::to_string(&Answer::refused("alreadySetUp")).unwrap(),
            r#"{"refused":{"reason":"alreadySetUp"}}"#
        );
    }
}
