//! Safe Mode (docs/adr/0054-recovery-and-erasing.mdx, decision 10): a start the person chose in
//! the startup picker, which runs the session with the system's own parts alone.
//!
//! The boot loader's *OxyOS Safe Mode* entry is its kernel's own with `oxyos.safe-mode=1` on the
//! command line; the session reads it once, as it starts, from the running kernel (`/proc/cmdline`),
//! which nothing running can change and which only the entry, on the EFI system partition root
//! alone writes, sets. What it leaves out is each part's own: the applications opened at login
//! (the session), background activity of every application but the image's
//! ([`crate::background::BackgroundActivity::set_safe_mode`]), and an input-method engine other than
//! the image's ([`crate::input_methods::for_session`]). Safe Mode writes nothing: the next start is
//! normal.

/// The kernel's parameter, as Safe Mode's entry writes it (`oxyos.safe-mode=1`).
pub const PARAMETER: &str = "oxyos.safe-mode";

/// Whether a kernel command line asks for Safe Mode: `oxyos.safe-mode` alone, or set to a true
/// value (`1`, `yes`, `y`, `true`, `on`), as systemd reads its own switches; the last one given
/// wins, as the kernel's parameters do. Anything else, or none, is a normal start.
pub fn from_command_line(line: &str) -> bool {
    let mut safe = false;
    for word in line.split_ascii_whitespace() {
        let (name, value) = match word.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (word, None),
        };
        if name == PARAMETER {
            safe = value.is_none_or(|value| matches!(value, "1" | "yes" | "y" | "true" | "on"));
        }
    }
    safe
}

/// Whether this start is Safe Mode: the running kernel's command line, read once. One that cannot
/// be read is a normal start.
pub fn this_start() -> bool {
    std::fs::read_to_string("/proc/cmdline").is_ok_and(|line| from_command_line(&line))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_mode_is_its_parameter_on_the_command_line_the_last_one_winning() {
        let normal = "BOOT_IMAGE=/oxyos/6.12/linux root=/dev/mapper/oxyos-root rw quiet splash";
        assert!(!from_command_line(normal));
        assert!(from_command_line(&format!("{normal} oxyos.safe-mode=1")));
        assert!(from_command_line("oxyos.safe-mode quiet"));
        for value in ["yes", "y", "true", "on"] {
            assert!(
                from_command_line(&format!("oxyos.safe-mode={value}")),
                "{value}"
            );
        }
        for value in ["0", "no", "false", "off", "", "2"] {
            assert!(
                !from_command_line(&format!("oxyos.safe-mode={value}")),
                "{value}"
            );
        }
        // The last given wins, as the kernel's parameters do.
        assert!(!from_command_line("oxyos.safe-mode=1 oxyos.safe-mode=0"));
        assert!(from_command_line("oxyos.safe-mode=0 oxyos.safe-mode=1\n"));
        // Only its own name: a longer or a prefixed one is another parameter.
        assert!(!from_command_line(
            "oxyos.safe-modes=1 xoxyos.safe-mode=1 oxyos.safe=1"
        ));
    }
}
