//! OxyOS's kernel-install plugin for Safe Mode (docs/adr/0054-recovery-and-erasing.mdx, decision
//! 10), run as kernel-install runs it: with `add` after the kernel's entry is written, Safe Mode's
//! is written beside it by this program; with `remove`, it goes; outside systemd-boot's layout,
//! nothing is done.

use std::fs;
use std::path::Path;
use std::process::Command;

const PLUGIN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/package/usr/lib/kernel/install.d/91-oxyos-safe-mode.install"
);
const VERSION: &str = "6.12.48+deb13-amd64";

fn run(boot_root: &Path, layout: &str, command: &str) -> std::process::Output {
    let program = Path::new(env!("CARGO_BIN_EXE_oxyos-recovery"));
    let path = format!(
        "{}:{}",
        program.parent().unwrap().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    Command::new("sh")
        .arg(PLUGIN)
        .args([
            command,
            VERSION,
            "/efi/oxyos/6.12.48+deb13-amd64",
            "/boot/vmlinuz-6.12.48+deb13-amd64",
        ])
        .env("PATH", path)
        .env("KERNEL_INSTALL_LAYOUT", layout)
        .env("KERNEL_INSTALL_BOOT_ROOT", boot_root)
        .env("KERNEL_INSTALL_ENTRY_TOKEN", "oxyos")
        .output()
        .unwrap()
}

#[test]
fn kernel_install_writes_safe_mode_beside_the_kernel_and_removes_it_with_it() {
    let esp = std::env::temp_dir().join(format!("oxyos-safe-mode-plugin-{}", std::process::id()));
    let _ = fs::remove_dir_all(&esp);
    let entries = esp.join("loader/entries");
    fs::create_dir_all(&entries).unwrap();
    fs::write(
        entries.join(format!("oxyos-{VERSION}+3.conf")),
        format!("title      OxyOS\nsort-key   debian\nversion    {VERSION}\noptions    root=/dev/mapper/oxyos-root rw quiet\nlinux      /oxyos/{VERSION}/linux\ninitrd     /oxyos/{VERSION}/initrd.img-{VERSION}\n"),
    )
    .unwrap();
    let safe = entries.join(format!("oxyos-safe-mode-{VERSION}.conf"));
    // Another layout (GRUB's, none) is not OxyOS's boot: nothing is written.
    assert!(run(&esp, "other", "add").status.success());
    assert!(!safe.exists());
    let added = run(&esp, "bls", "add");
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    let written = fs::read_to_string(&safe).unwrap();
    assert!(
        written.starts_with("title      OxyOS Safe Mode\nsort-key   oxyos-safe-mode\n"),
        "{written}"
    );
    assert!(
        written.contains("options    root=/dev/mapper/oxyos-root rw quiet oxyos.safe-mode=1\n"),
        "{written}"
    );
    assert!(written.contains(&format!("linux      /oxyos/{VERSION}/linux\n")));
    // The kernel's own is as it was.
    assert!(entries.join(format!("oxyos-{VERSION}+3.conf")).exists());
    let removed = run(&esp, "bls", "remove");
    assert!(
        removed.status.success(),
        "{}",
        String::from_utf8_lossy(&removed.stderr)
    );
    assert!(!safe.exists());
    fs::remove_dir_all(&esp).unwrap();
}

#[test]
fn sync_writes_the_installed_kernels_safe_mode_entries_once() {
    // As a deploy to a converted machine runs it (tools/deploy/remote.sh), with kernel-install's
    // boot root and entry token.
    let esp = std::env::temp_dir().join(format!("oxyos-safe-mode-sync-{}", std::process::id()));
    let _ = fs::remove_dir_all(&esp);
    let entries = esp.join("loader/entries");
    fs::create_dir_all(&entries).unwrap();
    fs::write(
        entries.join(format!("oxyos-{VERSION}+3.conf")),
        format!("title      OxyOS\nsort-key   oxyos\nversion    {VERSION}\noptions    root=UUID=0 ro quiet\nlinux      /oxyos/{VERSION}/linux\ninitrd     /oxyos/{VERSION}/initrd.img-{VERSION}\n"),
    )
    .unwrap();
    fs::write(
        entries.join("oxyos-recovery.conf"),
        "title      OxyOS Recovery\nsort-key   oxyos-recovery\n",
    )
    .unwrap();
    let sync = || {
        let output = Command::new(env!("CARGO_BIN_EXE_oxyos-recovery"))
            .args(["safe-mode-entry", "sync", "--boot-root"])
            .arg(&esp)
            .args(["--entry-token", "oxyos"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    assert_eq!(sync(), format!("oxyos-safe-mode-{VERSION}.conf written.\n"));
    assert!(
        fs::read_to_string(entries.join(format!("oxyos-safe-mode-{VERSION}.conf")))
            .unwrap()
            .contains("sort-key   oxyos-safe-mode\n")
    );
    assert_eq!(sync(), "Safe Mode's entries are as the kernels' are.\n");
    fs::remove_dir_all(&esp).unwrap();
}
