//! `oxyos-recovery`: OxyOS on the disk and getting it back
//! ([ADR 0054](../../../docs/adr/0054-recovery-and-erasing.mdx)). One program, run by root, for:
//!
//! - `install --developer --disk DISK [--no-encrypt] [--developer-user NAME]`: the development
//!   installer of [ADR 0023](../../../docs/adr/0023-installing-oxyos.mdx) (decision 7: until the
//!   installer application is in the image), from the live medium's console. It erases DISK
//!   whole, after the person types its name. Never on a published medium.
//! - `console`: the recovery, on the system console (`oxyos-recovery-console.service`, in
//!   `oxyos-recovery.target`), until its Bloom client is in the image: erase and reinstall, check
//!   the disk, help with a forgotten password, Wi-Fi, language, restart, shut down; and an erase
//!   the running system asked for, carried out as soon as it starts.
//! - `boot-tries`: after a complete boot, the entry that started the system gets its tries back
//!   (`oxyos-boot-tries.service`).
//! - `arm-erase`: leaves the erase request for the recovery and makes the next boot the
//!   recovery's (`oxyos-erase.service`, which the account service starts once an administrator
//!   confirmed *Erase All Content and Settings* with their password).

mod console;
mod messages;
mod terminal;

use oxyos_recovery::disks::{self, Unsuitable};
use oxyos_recovery::entries::{self, Entries};
use oxyos_recovery::host::{Command, Host, System};
use oxyos_recovery::install::{self, Developer, Options, Paths};
use oxyos_recovery::layout::{self, Image};
use oxyos_recovery::request::{self, EraseRequest, Found};
use std::path::Path;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

/// The installed system's EFI system partition.
const ESP: &str = "/efi";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let result = match arguments.first().map(String::as_str) {
        Some("install") => install(&arguments[1..]),
        Some("console") => console::run(),
        Some("boot-tries") => boot_tries(),
        Some("arm-erase") => arm_erase(),
        Some("recovery-entry") => recovery_entry(&arguments[1..]),
        Some("rearm") => rearm(&arguments[1..]),
        _ => Err("usage: oxyos-recovery install --developer --disk DISK [--no-encrypt] [--developer-user NAME] | console | boot-tries | arm-erase | recovery-entry [--system-root PARTUUID] | rearm --esp DIRECTORY".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("oxyos-recovery: {error}");
            ExitCode::FAILURE
        }
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// After a complete boot (`boot-complete.target`, once `systemd-bless-boot` marked it good): the
/// entry that booted gets its tries back, so the next three starts that never complete, whenever
/// they come, end in the recovery. Not while an erase is pending: its entries stay condemned.
fn boot_tries() -> Result<(), String> {
    let esp = Path::new(ESP);
    match request::read(esp).map_err(|e| format!("the erase request: {e}"))? {
        Found::None => {}
        _ => {
            println!(
                "An erase is pending: the recovery starts next, and the entries keep no tries."
            );
            return Ok(());
        }
    }
    let Some(booted) = entries::loader_string(
        Path::new("/sys/firmware/efi/efivars"),
        "LoaderEntrySelected",
    )
    .map_err(|e| format!("LoaderEntrySelected: {e}"))?
    else {
        println!("The system was not started by systemd-boot: nothing to count.");
        return Ok(());
    };
    let tries = std::fs::read_to_string("/etc/kernel/tries")
        .ok()
        .and_then(|t| t.trim().parse().ok())
        .filter(|&t| t > 0)
        .unwrap_or(entries::TRIES);
    match Entries::new(esp)
        .rearm(&booted, tries)
        .map_err(|e| format!("the boot entries: {e}"))?
    {
        Some(name) => println!("{booted} booted: it is {name} again."),
        None => println!("{booted} booted: nothing to give back."),
    }
    Ok(())
}

/// Arms the erase: the request for this disk and root, then every OxyOS entry condemned, so the
/// next boot, whatever starts it, is the recovery's, which carries the request out. Refuses, having
/// changed nothing, when the machine has no recovery to do it.
fn arm_erase() -> Result<(), String> {
    let mut host = System;
    let disks = install::disks(&mut host).map_err(|e| e.to_string())?;
    let disk = disks::disk_with_mount(&disks, "/").ok_or("no disk holds the running system")?;
    let layout = disk
        .layout()
        .ok_or("the running system's disk has no OxyOS recovery")?;
    if !layout.esp.mounted_at(ESP) {
        return Err(format!("the EFI system partition is not mounted at {ESP}"));
    }
    let esp = Path::new(ESP);
    if !esp
        .join("loader/entries")
        .join(layout::RECOVERY_ENTRY)
        .is_file()
    {
        return Err("the recovery has no boot entry".into());
    }
    let request = EraseRequest::arm(&layout, now())?;
    request::write(esp, &request).map_err(|e| format!("the erase request: {e}"))?;
    if let Err(e) = Entries::new(esp).condemn_all() {
        let _ = request::remove(esp);
        let _ = Entries::new(esp).rearm_all(entries::TRIES);
        return Err(format!("the boot entries: {e}"));
    }
    println!(
        "Erase armed for {} (root {}): the recovery erases it at the next boot.",
        disk.path, request.root
    );
    Ok(())
}

/// Writes the recovery's entry, kernel and initramfs onto the EFI system partition at /efi, from
/// the recovery partition of the disk that holds it. `--system-root PARTUUID` names a converted
/// machine's root (ADR 0054, decision 9).
fn recovery_entry(arguments: &[String]) -> Result<(), String> {
    let system_root = match arguments {
        [] => None,
        [flag, uuid] if flag == "--system-root" => {
            if disks::converted_root(&format!("oxyos.recovery.root=PARTUUID={uuid}")).is_none() {
                return Err(format!("{uuid} is not a partition UUID"));
            }
            Some(uuid.to_ascii_lowercase())
        }
        _ => return Err("usage: oxyos-recovery recovery-entry [--system-root PARTUUID]".into()),
    };
    let mut host = System;
    let disks = install::disks(&mut host).map_err(|e| e.to_string())?;
    let disk = disks::disk_with_mount(&disks, ESP).ok_or(format!("nothing is mounted at {ESP}"))?;
    let recovery = disk
        .only(layout::RECOVERY_TYPE)
        .ok_or(format!("{} has no OxyOS Recovery partition", disk.path))?;
    if let Some(root) = &system_root {
        disk.converted_layout(root).ok_or(format!(
            "{root} is not a root partition of {} beside its recovery",
            disk.path
        ))?;
    }
    let hash = install::add_recovery_entry(
        &mut host,
        &Paths::default(),
        Path::new(ESP),
        recovery,
        system_root.as_deref(),
    )
    .map_err(|e| e.to_string())?;
    println!(
        "OxyOS Recovery's entry written: {} on {}, root hash {hash}.",
        layout::RECOVERY_ENTRY,
        recovery.path
    );
    Ok(())
}

/// Gives every OxyOS entry on the EFI system partition at `--esp` its tries back, as the
/// recovery's Restart does.
fn rearm(arguments: &[String]) -> Result<(), String> {
    let [flag, esp] = arguments else {
        return Err("usage: oxyos-recovery rearm --esp DIRECTORY".into());
    };
    if flag != "--esp" {
        return Err("usage: oxyos-recovery rearm --esp DIRECTORY".into());
    }
    let changed = Entries::new(Path::new(esp))
        .rearm_all(entries::TRIES)
        .map_err(|e| e.to_string())?;
    println!("{changed} entries given their tries back.");
    Ok(())
}

/// The development installer.
fn install(arguments: &[String]) -> Result<(), String> {
    let mut developer_mode = false;
    let mut disk = None;
    let mut encrypt = true;
    let mut user = None;
    let mut rest = arguments.iter();
    while let Some(argument) = rest.next() {
        match argument.as_str() {
            "--developer" => developer_mode = true,
            "--disk" => disk = Some(rest.next().ok_or("--disk needs a disk")?.clone()),
            "--no-encrypt" => encrypt = false,
            "--developer-user" => {
                user = Some(rest.next().ok_or("--developer-user needs a name")?.clone())
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if !developer_mode {
        return Err("this is the development installer: it runs only with --developer (docs/adr/0023-installing-oxyos.mdx, decision 7)".into());
    }
    let disk = disk.ok_or("--disk DISK: the whole disk to erase and install onto")?;
    let image = Image::live_boot();
    if !Path::new(&image.squashfs()).is_file() {
        return Err("no live medium: run this from OxyOS's live system".into());
    }
    let mut host = System;
    let disks = install::disks(&mut host).map_err(|e| e.to_string())?;
    let chosen = disks
        .iter()
        .find(|d| d.path == disk)
        .ok_or_else(|| format!("{disk} is not a disk"))?;
    match disks::suitable(chosen, "/run/live/medium") {
        Ok(()) => {}
        Err(Unsuitable::BootMedium) => {
            return Err(format!("{disk} is the medium this system started from"))
        }
        Err(Unsuitable::ReadOnly) => return Err(format!("{disk} is read-only")),
        Err(Unsuitable::TooSmall { size }) => {
            return Err(format!(
                "{disk} has {:.1} GB; OxyOS needs {:.1} GB",
                size as f64 / 1e9,
                layout::MIN_DISK_SIZE as f64 / 1e9
            ))
        }
    }
    let developer = match user {
        Some(user) => {
            if !user
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
                || !user.starts_with(|c: char| c.is_ascii_lowercase())
                || user.len() > 32
            {
                return Err(format!(
                    "{user} is not a user name (lowercase letters, digits, - and _)"
                ));
            }
            let password = terminal::read_secret(&format!("Password for {user}: "))
                .map_err(|e| e.to_string())?;
            let again = terminal::read_secret("Again: ").map_err(|e| e.to_string())?;
            if password.is_empty() || *password != *again {
                return Err("the passwords are empty or differ".into());
            }
            Some(Developer { user, password })
        }
        None => None,
    };
    println!("This erases everything on {}.", chosen.describe());
    println!(
        "OxyOS is installed {}.",
        if encrypt {
            "encrypted"
        } else {
            "NOT encrypted"
        }
    );
    let typed = terminal::read_line(&format!("Type {disk} to erase it and install: "))
        .map_err(|e| e.to_string())?;
    if typed.trim() != disk {
        return Err("not confirmed: nothing was changed".into());
    }
    let options = Options { encrypt, developer };
    let installed = install::install(
        &mut host,
        &Paths::default(),
        &image,
        &disk,
        &options,
        &mut |step| {
            println!(
                "{}",
                messages::Text::new(messages::Language::English).step(step)
            );
        },
    )
    .map_err(|e| e.to_string())?;
    if let Some(key) = installed.recovery_key {
        println!("\nThe disk's recovery key (write it down; it unlocks the disk if the password is lost):\n\n    {}\n", *key);
    }
    println!("OxyOS is installed on {disk}. Remove the installation medium and restart.");
    Ok(())
}

/// Runs the system's own `systemctl` to restart or power off.
pub(crate) fn systemctl(host: &mut dyn Host, verb: &str) -> Result<(), String> {
    host.run(&Command::new("systemctl").arg(verb))
        .map(drop)
        .map_err(|e| e.to_string())
}
