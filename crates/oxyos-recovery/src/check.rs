//! Checking and repairing the disk from the recovery ([ADR 0054](../../../docs/adr/0054-recovery-and-erasing.mdx),
//! decision 5): the EFI system partition with fsck.vfat, the root with e2fsck, forced and repairing
//! what it can. An encrypted root is opened only with what opens it at boot: an account's password
//! or the recovery key. Nothing here can open it without one.

use crate::disks::Layout;
use crate::host::{failed, Command, Failed, Host};

/// What a check came to, for each file system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing wrong.
    Clean,
    /// Errors were found and repaired.
    Repaired,
    /// Errors remain.
    Damaged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report {
    pub esp: Outcome,
    pub root: Outcome,
}

/// Why a check did not run.
#[derive(Debug)]
pub enum Refused {
    /// The secret opens no keyslot.
    WrongSecret,
    Failed(Failed),
}

impl From<Failed> for Refused {
    fn from(failed: Failed) -> Self {
        Refused::Failed(failed)
    }
}

/// e2fsck's and fsck.vfat's exit status: 0 clean, 1 (and 2, reboot advised) repaired, anything
/// else left damaged or not checked.
pub fn outcome(status: i32) -> Outcome {
    match status {
        0 => Outcome::Clean,
        1 | 2 => Outcome::Repaired,
        _ => Outcome::Damaged,
    }
}

const MAPPING: &str = "oxyos-check";

/// Checks and repairs `layout`'s file systems; `secret` opens an encrypted root.
pub fn check(
    host: &mut dyn Host,
    layout: &Layout,
    secret: Option<&[u8]>,
) -> Result<Report, Refused> {
    let esp = outcome(
        host.execute(&Command::new("fsck.vfat").args(["-a", "-w", &layout.esp.path]))?
            .status,
    );
    let root = if layout.root.encrypted() {
        let secret = secret.ok_or_else(|| failed("the disk is encrypted", "no password given"))?;
        let opened = host.execute(
            &Command::new("cryptsetup")
                .args(["open", "--key-file=-", &layout.root.path, MAPPING])
                .input(secret),
        )?;
        match opened.status {
            0 => {}
            // cryptsetup(8): 2, no keyslot opens with what was given.
            2 => return Err(Refused::WrongSecret),
            status => {
                return Err(failed(
                    format!("cryptsetup open failed ({status})"),
                    opened.stderr.trim(),
                )
                .into())
            }
        }
        let checked = host.execute(&Command::new("e2fsck").args([
            "-f",
            "-y",
            &format!("/dev/mapper/{MAPPING}"),
        ]));
        host.run(&Command::new("cryptsetup").args(["close", MAPPING]))?;
        outcome(checked?.status)
    } else {
        outcome(
            host.execute(&Command::new("e2fsck").args(["-f", "-y", &layout.root.path]))?
                .status,
        )
    };
    Ok(Report { esp, root })
}
