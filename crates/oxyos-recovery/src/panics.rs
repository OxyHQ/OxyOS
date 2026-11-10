//! "Your computer restarted because of a problem" (docs/adr/0068-crash-evidence-and-kernel-panics.mdx,
//! decision 3), as a Mac says it after a kernel panic. The kernel panics, restarts by itself
//! (`kernel.panic`), and the panic's text survives the restart in the firmware (efi-pstore), which
//! `systemd-pstore` moves into `/var/lib/systemd/pstore` at the next boot, root's alone. This part,
//! as root at boot (`oxyos-recovery panic-report`), writes each new one's account for the person:
//! `/var/lib/oxyos/problems/<record>.panic`, readable by everyone, which the person's session tells
//! them of once.
//!
//! What the person may read of a kernel's log is what says what happened (the panic's reason, the
//! oops, the code that failed, by name) and never an address: `kernel.dmesg_restrict` keeps the
//! kernel's log from unprivileged users because its addresses defeat the kernel's address layout
//! randomisation, so every long hexadecimal number is replaced, and the registers' lines left out.

use std::io;
use std::path::{Path, PathBuf};

/// Where systemd-pstore keeps what the firmware held.
pub const PSTORE: &str = "/var/lib/systemd/pstore";
/// Where the accounts for the person are written.
pub const PROBLEMS: &str = "/var/lib/oxyos/problems";
/// The longest account written.
pub const MAX_BYTES: usize = 8 * 1024;

/// The lines of a kernel's log that say what happened, from its first sign of a problem.
const MARKS: [&str; 6] = [
    "Kernel panic",
    "Oops",
    "BUG:",
    "general protection fault",
    "Call Trace:",
    "watchdog:",
];

/// Lines that hold the machine's state, never its story: registers and raw memory.
const LEFT_OUT: [&str; 9] = [
    "RSP:", "RAX:", "RDX:", "RBP:", "R10:", "R13:", "FS:", "CS:", "CR2:",
];

/// A record of systemd-pstore's: a folder of its own (a record's ID, efi-pstore's) with the
/// kernel's log rebuilt as `dmesg.txt`, or a `dmesg-*` file of its own.
fn records(pstore: &Path) -> io::Result<Vec<(String, PathBuf)>> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(pstore)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            let log = entry.path().join("dmesg.txt");
            if log.is_file() {
                found.push((name, log));
            }
        } else if kind.is_file() && name.starts_with("dmesg") {
            found.push((name, entry.path()));
        }
    }
    found.sort();
    Ok(found)
}

/// Every long hexadecimal number (an address, a register's value) replaced by `…`.
fn without_addresses(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut digits = String::new();
    let flush = |digits: &mut String, out: &mut String| {
        let number = digits.trim_start_matches("0x");
        if number.len() >= 8 {
            out.push('…');
        } else {
            out.push_str(digits);
        }
        digits.clear();
    };
    for c in line.chars() {
        if c.is_ascii_hexdigit() || (c == 'x' && digits == "0") {
            digits.push(c);
        } else {
            flush(&mut digits, &mut out);
            out.push(c);
        }
    }
    flush(&mut digits, &mut out);
    out
}

/// The account of a kernel's log for the person: from its first sign of a problem, the lines that
/// say what happened, without addresses or registers, at most [`MAX_BYTES`]. None when the log
/// shows no problem (a record of an orderly restart, which efi-pstore does not keep anyway).
pub fn account(log: &str) -> Option<String> {
    let start = log
        .lines()
        .position(|line| MARKS.iter().any(|mark| line.contains(mark)))?;
    let mut text = String::from(
        "Your computer restarted because of a problem.\n\nThe kernel's last words before it restarted:\n\n",
    );
    for line in log.lines().skip(start) {
        // The log's own prefixes (a record's header, "<0>[ 12.345678]") are kept as they are.
        if LEFT_OUT.iter().any(|left| line.contains(left)) || line.contains("Code: ") {
            continue;
        }
        let line = without_addresses(line);
        if text.len() + line.len() + 1 > MAX_BYTES {
            text.push_str("…\n");
            break;
        }
        text.push_str(&line);
        text.push('\n');
    }
    Some(text)
}

/// Writes the account of each new record in `pstore` into `problems` (made 0755, each 0644,
/// written whole and never replaced): the names written.
pub fn report(pstore: &Path, problems: &Path) -> io::Result<Vec<String>> {
    use std::os::unix::fs::PermissionsExt;
    let mut written = Vec::new();
    let found = match records(pstore) {
        Ok(found) => found,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(written),
        Err(error) => return Err(error),
    };
    for (name, log) in found {
        let target = problems.join(format!("{name}.panic"));
        if target.exists() {
            continue;
        }
        let bytes = std::fs::read(&log)?;
        let Some(text) = account(&String::from_utf8_lossy(&bytes)) else {
            continue;
        };
        std::fs::create_dir_all(problems)?;
        std::fs::set_permissions(problems, std::fs::Permissions::from_mode(0o755))?;
        let partial = problems.join(format!(".{name}.panic.new"));
        std::fs::write(&partial, text)?;
        std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o644))?;
        std::fs::rename(&partial, &target)?;
        written.push(name);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANIC: &str = "<6>[    0.000000] Linux version 6.12.101+deb13-amd64
<4>[  812.330000] sysrq: Trigger a crash
<0>[  812.330101] Kernel panic - not syncing: sysrq triggered crash
<4>[  812.330200] CPU: 3 UID: 0 PID: 2211 Comm: sh Not tainted 6.12.101+deb13-amd64 #1
<4>[  812.330300] Call Trace:
<4>[  812.330301]  <TASK>
<4>[  812.330302]  dump_stack_lvl+0x5d/0x80
<4>[  812.330303]  panic+0x118/0x2e0
<4>[  812.330304]  sysrq_handle_crash+0x1a/0x20
<4>[  812.330400] RIP: 0010:sysrq_handle_crash+0x1a/0x20 at ffffffffa1b2c3d4
<4>[  812.330401] RSP: 0018:ffffb3c0c1e8fd88 EFLAGS: 00010246
<4>[  812.330402] RAX: 0000000000000000 RBX: 0000000000000063 RCX: 0000000000000000
<4>[  812.330403] Code: 48 89 e5 c7 05 c1 9b 75 01 01 00 00 00
<0>[  812.330500] Kernel Offset: 0x20000000 from 0xffffffff81000000
";

    #[test]
    fn a_panics_account_says_what_happened_and_no_address() {
        let text = account(PANIC).unwrap();
        assert!(text.starts_with("Your computer restarted because of a problem."));
        assert!(text.contains("Kernel panic - not syncing: sysrq triggered crash"));
        assert!(text.contains("sysrq_handle_crash+0x1a/0x20"));
        assert!(
            !text.contains("Linux version"),
            "from its first sign of a problem"
        );
        for leaked in [
            "ffffffffa1b2c3d4",
            "ffffb3c0c1e8fd88",
            "ffffffff81000000",
            "RAX:",
            "Code:",
        ] {
            assert!(!text.contains(leaked), "{leaked} in {text}");
        }
        assert!(text.contains("RIP: 0010:sysrq_handle_crash+0x1a/0x20 at …"));
        assert!(text.contains("Kernel Offset: … from …"));
        // A log with no problem in it is no account; a long one is cut.
        assert_eq!(account("<6>[ 1.0] all is well\n"), None);
        let long = format!("Oops: 0000\n{}", "x\n".repeat(10_000));
        assert!(account(&long).unwrap().len() <= MAX_BYTES + 4);
    }

    #[test]
    fn each_new_record_is_written_once_for_everyone_to_read() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("oxyos-panics-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (pstore, problems) = (root.join("pstore"), root.join("problems"));
        std::fs::create_dir_all(pstore.join("1762790000000000001")).unwrap();
        std::fs::write(pstore.join("1762790000000000001/dmesg.txt"), PANIC).unwrap();
        std::fs::write(pstore.join("dmesg-efi-176279"), "nothing wrong\n").unwrap();
        assert_eq!(report(&pstore, &problems).unwrap(), ["1762790000000000001"]);
        let written = problems.join("1762790000000000001.panic");
        let mode = std::fs::metadata(&written).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o644);
        assert!(std::fs::read_to_string(&written)
            .unwrap()
            .contains("sysrq triggered crash"));
        // Again: nothing new.
        assert!(report(&pstore, &problems).unwrap().is_empty());
        // No pstore at all (no record ever kept): nothing, no error.
        assert!(report(&root.join("none"), &problems).unwrap().is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
