//! A share on a NAS or another computer, reached over SMB through rclone's own SMB client, which
//! restic starts as its `rclone:` backend ([ADR 0059](../../docs/adr/0059-backups.mdx), decision
//! 3). Nothing is mounted: a share that goes away (the laptop leaves home mid-backup) fails the
//! backup with a network error, where a kernel mount would leave processes waiting on it.
//!
//! rclone is configured by environment variables alone (`RCLONE_CONFIG_<REMOTE>_<OPTION>`), its
//! configuration file unused; its password option is rclone's obscured form, made by rclone's own
//! `rclone obscure`, reading the password on its standard input.

use crate::engine::Location;
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use zeroize::Zeroizing;

/// The remote's name in rclone's environment.
const REMOTE: &str = "OXYOSSHARE";

/// A share, as the person named it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Share {
    /// The computer: a host name or an address.
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// The share's name.
    pub share: String,
    /// The user on that computer.
    pub user: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

impl Share {
    /// What the person may be shown: `smb://host/share`.
    pub fn url(&self) -> String {
        match self.port {
            Some(port) => format!("smb://{}:{port}/{}", self.host, self.share),
            None => format!("smb://{}/{}", self.host, self.share),
        }
    }

    /// Checks what the person typed: a host and a share's name that rclone reads as such.
    pub fn validate(&self) -> Result<(), &'static str> {
        let host_ok = !self.host.is_empty()
            && self.host.len() <= 253
            && self
                .host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'));
        let share_ok = !self.share.is_empty()
            && self.share.len() <= 80
            && !self
                .share
                .chars()
                .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'));
        let user_ok = !self.user.is_empty()
            && self.user.len() <= 256
            && !self.user.chars().any(char::is_control);
        let domain_ok = self
            .domain
            .as_ref()
            .is_none_or(|d| d.len() <= 256 && !d.chars().any(char::is_control));
        match (host_ok, share_ok, user_ok, domain_ok) {
            (false, ..) => Err("invalidHost"),
            (_, false, ..) => Err("invalidShare"),
            (_, _, false, _) => Err("invalidUser"),
            (.., false) => Err("invalidDomain"),
            _ => Ok(()),
        }
    }

    /// Where a repository at `folder` of the share is, restic's `rclone:` location, the share's
    /// password `obscured` as rclone takes it.
    pub fn location(&self, folder: &str, obscured: &str) -> Location {
        let mut environment = vec![
            (
                format!("RCLONE_CONFIG_{REMOTE}_TYPE"),
                "smb".to_owned().into(),
            ),
            (
                format!("RCLONE_CONFIG_{REMOTE}_HOST"),
                self.host.clone().into(),
            ),
            (
                format!("RCLONE_CONFIG_{REMOTE}_USER"),
                self.user.clone().into(),
            ),
            (
                format!("RCLONE_CONFIG_{REMOTE}_PASS"),
                Zeroizing::new(obscured.to_owned()),
            ),
            // rclone's configuration file is never read.
            ("RCLONE_CONFIG".to_owned(), "/dev/null".to_owned().into()),
        ];
        if let Some(port) = self.port {
            environment.push((
                format!("RCLONE_CONFIG_{REMOTE}_PORT"),
                port.to_string().into(),
            ));
        }
        if let Some(domain) = &self.domain {
            environment.push((
                format!("RCLONE_CONFIG_{REMOTE}_DOMAIN"),
                domain.clone().into(),
            ));
        }
        Location {
            repository: format!("rclone:{}:{}/{folder}", REMOTE.to_lowercase(), self.share).into(),
            environment,
        }
    }
}

/// The password as rclone's configuration takes it (`rclone obscure`, the password on its
/// standard input).
pub fn obscure(rclone: &Path, password: &str) -> io::Result<Zeroizing<String>> {
    let mut child = Command::new(rclone)
        .args(["obscure", "-"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("RCLONE_CONFIG", "/dev/null")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(password.as_bytes())?;
    }
    let mut output = Zeroizing::new(String::new());
    if let Some(mut stdout) = child.stdout.take() {
        stdout.read_to_string(&mut output)?;
    }
    let status = child.wait()?;
    if !status.success() || output.trim().is_empty() {
        return Err(io::Error::other("rclone could not take the password"));
    }
    Ok(Zeroizing::new(output.trim().to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn share() -> Share {
        Share {
            host: "nas.local".into(),
            port: None,
            share: "Backups".into(),
            user: "ada".into(),
            domain: None,
        }
    }

    #[test]
    fn a_share_is_restics_rclone_location_configured_by_its_environment() {
        let location = share().location("OxyOS Backups/abc", "OBSCURED");
        assert_eq!(
            location.repository,
            "rclone:oxyosshare:Backups/OxyOS Backups/abc"
        );
        let names: Vec<&str> = location
            .environment
            .iter()
            .map(|(n, _)| n.as_str())
            .collect();
        assert!(names.contains(&"RCLONE_CONFIG_OXYOSSHARE_TYPE"));
        assert!(names.contains(&"RCLONE_CONFIG_OXYOSSHARE_PASS"));
        assert!(!names.contains(&"RCLONE_CONFIG_OXYOSSHARE_PORT"));
        assert_eq!(share().url(), "smb://nas.local/Backups");
    }

    #[test]
    fn what_the_person_typed_is_checked() {
        assert_eq!(share().validate(), Ok(()));
        let mut bad = share();
        bad.host = "nas local".into();
        assert_eq!(bad.validate(), Err("invalidHost"));
        let mut bad = share();
        bad.share = "a/b".into();
        assert_eq!(bad.validate(), Err("invalidShare"));
        let mut bad = share();
        bad.user = String::new();
        assert_eq!(bad.validate(), Err("invalidUser"));
    }
}
