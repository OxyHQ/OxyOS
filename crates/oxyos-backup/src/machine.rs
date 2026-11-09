//! The machine's manifest ([ADR 0059](../../docs/adr/0059-backups.mdx), decision 6): what a
//! restore needs to know of the computer a backup is of, written by the service before each
//! backup into a folder every backup reads (`/var/lib/oxyos/backup/manifest/machine.json`).
//!
//! It lists the enrolled accounts as the login screen does (their account, user, uid and name)
//! and which were administrators, never a credential, a verifier or a keychain: a restored or
//! migrated account is set up again online, as one Oxy signed out is (ADR 0020, `signInAgain`),
//! and its keychain comes back from Oxy (ADR 0050). A machine's device credentials are its own and
//! never move to another computer in a backup.

use oxyos_account::store::Store as Accounts;
use oxyos_account::users::{group_members, passwd_entry};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// The manifest's file in its folder.
pub const MANIFEST: &str = "machine.json";
const VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub version: u32,
    /// The computer's name in its snapshots.
    pub computer: String,
    /// When it was written.
    pub written: i64,
    pub accounts: Vec<Account>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub account_id: String,
    pub user: String,
    pub uid: u32,
    /// Its group's number, which its files' group is: absent from manifests written before it was
    /// added, where a restore gives the group the uid's number, as useradd's user groups have it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gid: Option<u32>,
    pub name: String,
    pub administrator: bool,
}

impl Manifest {
    /// The machine's, from the account service's directory and the user and group files.
    pub fn of_machine(
        computer: &str,
        accounts: &Path,
        passwd: &Path,
        group: &Path,
        now: i64,
    ) -> io::Result<Self> {
        let administrators = group_members(group, "sudo")
            .map_err(|e| io::Error::other(e.to_string()))?
            .unwrap_or_default();
        let accounts = if accounts.is_dir() {
            let (listed, _) = Accounts::open(accounts)?.list()?;
            listed
                .into_iter()
                .map(|a| Account {
                    account_id: a.account.as_str().to_owned(),
                    administrator: administrators.iter().any(|m| m == a.user.as_str()),
                    gid: passwd_entry(passwd, a.user.as_str())
                        .ok()
                        .flatten()
                        .filter(|entry| entry.uid == a.uid)
                        .map(|entry| entry.gid),
                    user: a.user.as_str().to_owned(),
                    uid: a.uid,
                    name: a.name,
                })
                .collect()
        } else {
            Vec::new()
        };
        Ok(Self {
            version: VERSION,
            computer: computer.to_owned(),
            written: now,
            accounts,
        })
    }

    /// Writes it whole into `directory` (made, mode 0700, if it is not there).
    pub fn write(&self, directory: &Path) -> io::Result<()> {
        use std::os::unix::fs::DirBuilderExt;
        match std::fs::DirBuilder::new().mode(0o700).create(directory) {
            Err(error) if error.kind() != io::ErrorKind::AlreadyExists => return Err(error),
            _ => {}
        }
        let mut bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        bytes.push(b'\n');
        crate::store::write_whole(&directory.join(MANIFEST), &bytes)
    }

    /// A manifest as a backup holds it.
    pub fn read(bytes: &[u8]) -> io::Result<Self> {
        let manifest: Manifest = serde_json::from_slice(bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if manifest.version != VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("a manifest of version {}", manifest.version),
            ));
        }
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_machine_without_accounts_has_an_empty_manifest_written_whole() {
        let dir = tempfile::tempdir().unwrap();
        let group = dir.path().join("group");
        std::fs::write(&group, "root:x:0:\nsudo:x:27:oxy-a\n").unwrap();
        let passwd = dir.path().join("passwd");
        std::fs::write(&passwd, "root:x:0:0:root:/root:/bin/sh\n").unwrap();
        let manifest =
            Manifest::of_machine("ada-laptop", &dir.path().join("none"), &passwd, &group, 42)
                .unwrap();
        assert!(manifest.accounts.is_empty());
        let folder = dir.path().join("manifest");
        manifest.write(&folder).unwrap();
        let read = Manifest::read(&std::fs::read(folder.join(MANIFEST)).unwrap()).unwrap();
        assert_eq!(read, manifest);
        assert!(
            Manifest::read(br#"{"version":9,"computer":"","written":0,"accounts":[]}"#).is_err()
        );
    }
}
