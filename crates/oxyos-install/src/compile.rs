//! Compiling a package's JavaScript on the machine (docs/adr/0030-installing-applications.mdx,
//! decisions 1 and 5): Hermes runs bytecode as trusted input, so the bytecode that runs is always
//! the runtime's own `hermesc`'s output from the source the developer signed.
//!
//! The compiler parses untrusted JavaScript, so it runs confined: it is given the source on its
//! standard input and writes bytecode to its standard output (`hermesc -emit-binary -O -`), and
//! reaches nothing else. As root (the service) it runs as a transient systemd unit
//! (`systemd-run --wait --pipe`): a dynamic user, no network, no devices, the file system
//! read-only, no capabilities, `@system-service`'s system calls but no socket of any family, at
//! most ten minutes and 2 GiB.
//! systemd's own sandbox is the one every unit of the system is confined by, and a transient unit
//! needs no unit file the install service could get wrong; `--pipe` hands it the two descriptors.
//! Not as root (the tests), it runs directly.

use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

/// What compiles a bundle.
pub trait Compile {
    /// Compiles `source` (JavaScript) into `out` (empty) with the compiler at `hermesc`.
    fn compile(&self, hermesc: &Path, source: &File, out: &File) -> Result<(), String>;
}

/// The runtime's `hermesc`, confined when `confined`.
#[derive(Clone, Copy, Debug)]
pub struct Hermesc {
    pub confined: bool,
}

impl Hermesc {
    /// Confined if this process is root.
    pub fn for_this_process() -> Self {
        Self {
            confined: rustix::process::geteuid().is_root(),
        }
    }
}

/// The transient unit's sandbox.
const SANDBOX: &[&str] = &[
    "DynamicUser=yes",
    "PrivateNetwork=yes",
    "PrivateDevices=yes",
    "PrivateTmp=yes",
    "PrivateIPC=yes",
    "ProtectSystem=strict",
    "ProtectHome=yes",
    "ProtectProc=invisible",
    "ProcSubset=pid",
    "ProtectKernelTunables=yes",
    "ProtectKernelModules=yes",
    "ProtectKernelLogs=yes",
    "ProtectControlGroups=yes",
    "ProtectClock=yes",
    "ProtectHostname=yes",
    "NoNewPrivileges=yes",
    "CapabilityBoundingSet=",
    "RestrictNamespaces=yes",
    "RestrictRealtime=yes",
    "RestrictSUIDSGID=yes",
    "LockPersonality=yes",
    "MemoryDenyWriteExecute=yes",
    "SystemCallArchitectures=native",
    "SystemCallFilter=@system-service",
    // No socket of any family: `socket()`, `connect()` and the rest fail with EPERM. Not
    // `RestrictAddressFamilies=none`, which a unit file takes but systemd (257) refuses as a
    // transient unit's property ("Invalid argument"), so the unit never starts; this is stricter,
    // as no family is left at all.
    "SystemCallFilter=~@network-io",
    "SystemCallErrorNumber=EPERM",
    "UMask=0077",
    "RuntimeMaxSec=600",
    "MemoryMax=2G",
];

/// `program` as the compiler runs as root: a transient systemd unit with [`SANDBOX`]'s
/// properties, waited for, given this process's standard input and output (`--pipe`).
pub fn confined(program: &Path) -> Command {
    let mut command = Command::new("systemd-run");
    command.args([
        "--quiet",
        "--wait",
        "--pipe",
        "--collect",
        "--service-type=exec",
    ]);
    for property in SANDBOX {
        command.arg(format!("--property={property}"));
    }
    command.arg("--").arg(program);
    command
}

impl Compile for Hermesc {
    fn compile(&self, hermesc: &Path, source: &File, out: &File) -> Result<(), String> {
        let mut command = if self.confined {
            confined(hermesc)
        } else {
            Command::new(hermesc)
        };
        command
            .args(["-emit-binary", "-O", "-"])
            .stdin(Stdio::from(source.try_clone().map_err(|e| e.to_string())?))
            .stdout(Stdio::from(out.try_clone().map_err(|e| e.to_string())?))
            .stderr(Stdio::piped())
            .env_clear();
        let mut child = command
            .spawn()
            .map_err(|e| format!("{}: {e}", hermesc.display()))?;
        // What it says is read to its end (a compiler blocked on a full pipe would never exit),
        // its first 4 KiB kept.
        let mut said = Vec::new();
        if let Some(mut stderr) = child.stderr.take() {
            let mut buffer = [0; 4096];
            while let Ok(n) = stderr.read(&mut buffer) {
                if n == 0 {
                    break;
                }
                let room = 4096usize.saturating_sub(said.len());
                said.extend_from_slice(&buffer[..n.min(room)]);
            }
        }
        let said = String::from_utf8_lossy(&said);
        let status = child.wait().map_err(|e| e.to_string())?;
        if !status.success() {
            let said = said.lines().next().unwrap_or("").to_string();
            return Err(format!("the compiler failed ({status}): {said}"));
        }
        Ok(())
    }
}
