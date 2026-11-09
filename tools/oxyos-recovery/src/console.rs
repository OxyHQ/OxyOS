//! The recovery on the system console ([ADR 0054](../../../docs/adr/0054-recovery-and-erasing.mdx),
//! decision 5), until its Bloom client is in the image: the same choices, as text.
//!
//! At its start it finds the disk the recovery was started from and mounts that disk's EFI system
//! partition, then:
//! - an erase request for this disk and root is carried out at once, as was confirmed in Settings,
//!   and the computer restarts into setup;
//! - a request for another disk or root, or one that cannot be read, erases nothing: it is removed
//!   and the system's entries get their tries back;
//! - when every OxyOS entry has run out of tries, it says OxyOS could not start.
//!
//! *Restore from Backup…* ([ADR 0059](../../../docs/adr/0059-backups.mdx), decision 7) erases and
//! reinstalls as *Erase and Reinstall OxyOS* does, through the same request, which names the backup
//! (never its password), and restores it into the new root before the request is removed: a power
//! cut resumes it, the backup's password asked for again. Not offered on a converted machine, which
//! the recovery never erases (ADR 0054, decision 9).

use crate::messages::{Choice, Language, Text};
use crate::terminal::{read_line, read_secret};
use oxyos_backup::disks::{self as backup_disks, Disks};
use oxyos_backup::engine::{Engine, Repository, Stop};
use oxyos_backup::restore::{Folder, Refusal, Restorer, Source, Step as RestoreStep};
use oxyos_backup::share::Share;
use oxyos_recovery::check::{self, Refused};
use oxyos_recovery::disks::{self, Kind, Layout};
use oxyos_recovery::entries::{Entries, TRIES};
use oxyos_recovery::host::{failed, Failed};
use oxyos_recovery::host::{Command, Host, System};
use oxyos_recovery::install::{self, Paths};
use oxyos_recovery::layout::Image;
use oxyos_recovery::network;
use oxyos_recovery::request::{self, EraseRequest, Found, Restore, Verdict};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use zeroize::Zeroizing;

/// Where the recovery mounts its disk's EFI system partition.
const ESP: &str = "/run/oxyos-recovery/esp";
/// What live-boot mounted the recovery from.
const MEDIUM: &str = "/run/live/medium";
/// Where a backup disk is mounted, and restic's cache, on the recovery's RAM file system.
const BACKUP_VOLUMES: &str = "/run/oxyos-recovery/volumes";
const BACKUP_CACHE: &str = "/run/oxyos-recovery/restic";

/// A backup chosen to restore: its source opened (a disk stays mounted while this lives), its
/// repository and snapshot, as the request names them.
struct Chosen {
    _folder: Folder,
    repository: Repository,
    restore: Restore,
}

struct Recovery {
    host: System,
    text: Text,
    layout: Option<Layout>,
    esp: PathBuf,
}

pub fn run() -> Result<(), String> {
    let mut recovery = Recovery {
        host: System,
        text: Text::new(Language::English),
        layout: None,
        esp: ESP.into(),
    };
    recovery.find_disk();
    println!("\n{}\n", recovery.text.title());
    if recovery.layout.is_none() {
        println!("{}\n", recovery.text.no_layout());
    } else if recovery
        .layout
        .as_ref()
        .is_some_and(|l| l.kind == Kind::Converted)
    {
        println!("{}\n", recovery.text.converted());
    } else if recovery.carry_out_request() {
        return Ok(());
    }
    if recovery.system_cannot_start() {
        println!("{}\n", recovery.text.could_not_start());
    }
    recovery.menu()
}

impl Recovery {
    /// The disk holding the recovery this was started from, and its EFI system partition mounted.
    fn find_disk(&mut self) {
        let Ok(disks) = install::disks(&mut self.host) else {
            return;
        };
        let Some(disk) = disks::disk_with_mount(&disks, MEDIUM) else {
            eprintln!("oxyos-recovery: no disk holds {MEDIUM}");
            return;
        };
        // An install's layout, by its partitions' types; else a converted machine's, its root named
        // by the recovery's entry.
        let command_line = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
        let Some(layout) = disk.layout().or_else(|| {
            disks::converted_root(&command_line).and_then(|root| disk.converted_layout(root))
        }) else {
            eprintln!("oxyos-recovery: {} has no OxyOS layout", disk.path);
            return;
        };
        if !layout.recovery.mounted_at(MEDIUM) {
            eprintln!(
                "oxyos-recovery: the recovery started is not {}'s",
                disk.path
            );
            return;
        }
        let _ = std::fs::create_dir_all(&self.esp);
        // Read-write, an install's and a converted machine's alike: systemd-boot's entries are
        // there, and Restart gives them their tries back.
        let mounted = layout.esp.mounted_at(ESP)
            || self
                .host
                .run(&Command::new("mount").args([&layout.esp.path, ESP]))
                .is_ok();
        if mounted {
            eprintln!(
                "oxyos-recovery: {:?} layout on {}: EFI {}, recovery {}, root {}",
                layout.kind, disk.path, layout.esp.path, layout.recovery.path, layout.root.path
            );
            self.layout = Some(layout);
        }
    }

    fn system_cannot_start(&self) -> bool {
        self.layout.is_some()
            && Entries::new(&self.esp)
                .system_may_start()
                .is_ok_and(|may| !may)
    }

    /// Carries out the erase request, if there is one for this disk. True when the computer is
    /// restarting.
    fn carry_out_request(&mut self) -> bool {
        // A converted machine is never erased: no request is read there.
        let Some(layout) = self.layout.clone().filter(|l| l.kind == Kind::Installed) else {
            return false;
        };
        let mut request = match request::read(&self.esp) {
            Ok(Found::None) => return false,
            Ok(Found::Request(request)) if request.applies_to(&layout) == Verdict::Proceed => {
                request
            }
            Ok(Found::Request(_)) | Ok(Found::Unreadable(_)) | Err(_) => {
                // Not this disk's, or not readable: it erases nothing, and leaves nothing condemned.
                println!("{}\n", self.text.ignored_request());
                let _ = request::remove(&self.esp);
                let _ = Entries::new(&self.esp).rearm_all(TRIES);
                return false;
            }
        };
        if let Some(restore) = request.restore.clone() {
            return self.resume_restore(&layout, &mut request, &restore);
        }
        println!("{}\n", self.text.pending_erase());
        self.erase(&layout, &mut request)
    }

    /// Erases and installs again; true when it is done and the computer restarts.
    fn erase(&mut self, layout: &Layout, request: &mut EraseRequest) -> bool {
        self.erase_and_restore(layout, request, None)
    }

    /// Erases and installs again, then restores `chosen` into the new root if there is one; true
    /// when it is done and the computer restarts.
    fn erase_and_restore(
        &mut self,
        layout: &Layout,
        request: &mut EraseRequest,
        chosen: Option<(&Restorer, &Chosen)>,
    ) -> bool {
        let text = self.text;
        let mut names = String::new();
        let mut fill = |root: &Path| -> Result<(), Failed> {
            let Some((restorer, chosen)) = chosen else {
                return Ok(());
            };
            println!("{}", text.restoring());
            let mut shown = None;
            let restored = restorer
                .restore(
                    &chosen.repository,
                    &chosen.restore.snapshot,
                    root,
                    &Stop::new(),
                    |step| {
                        if let RestoreStep::Files(progress) = step {
                            let percent = (progress.percent_done.clamp(0.0, 1.0) * 100.0) as u32;
                            // A line every tenth, as the console scrolls.
                            if shown.is_none_or(|last: u32| percent / 10 > last / 10) {
                                println!("{}", text.restoring_files(percent));
                                shown = Some(percent);
                            }
                        }
                    },
                )
                .map_err(|refusal| failed(text.refusal(refusal.reason()), refusal.to_string()))?;
            names = restored
                .accounts
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Ok(())
        };
        let result = install::erase_and_fill(
            &mut self.host,
            &Paths::default(),
            &Image::live_boot(),
            &self.esp,
            layout,
            request,
            Some(&mut fill),
            &mut |step| {
                println!("{}", text.step(step));
            },
        );
        match result {
            Ok(()) => {
                if chosen.is_some() {
                    println!("\n{}", text.restored(&names));
                } else {
                    println!("\n{}", text.erased());
                }
                println!("{}", text.restarting());
                let _ = crate::systemctl(&mut self.host, "reboot");
                true
            }
            Err(error) => {
                println!("\n{}\n{} {error}\n", text.erase_failed(), text.failed());
                false
            }
        }
    }

    /// What the menu offers: Restore from Backup only where the recovery may erase.
    fn choices(&self) -> Vec<Choice> {
        let mut choices = vec![Choice::Erase];
        if self
            .layout
            .as_ref()
            .is_some_and(|l| l.kind == Kind::Installed)
        {
            choices.push(Choice::Restore);
        }
        choices.extend([
            Choice::Check,
            Choice::Forgot,
            Choice::Wifi,
            Choice::Language,
            Choice::Restart,
            Choice::ShutDown,
        ]);
        choices
    }

    fn menu(&mut self) -> Result<(), String> {
        loop {
            println!("{}\n", self.text.title());
            let choices = self.choices();
            for (number, choice) in choices.iter().enumerate() {
                println!("  {}  {}", number + 1, self.text.choice(*choice));
            }
            println!();
            let typed = read_line(self.text.choose()).map_err(|e| e.to_string())?;
            println!();
            let Some(choice) = typed
                .trim()
                .parse::<usize>()
                .ok()
                .and_then(|n| choices.get(n.wrapping_sub(1)))
            else {
                continue;
            };
            match choice {
                Choice::Erase => {
                    if self.erase_and_reinstall()? {
                        return Ok(());
                    }
                }
                Choice::Restore => {
                    if self.restore_from_backup()? {
                        return Ok(());
                    }
                }
                Choice::Check => self.check_disk()?,
                Choice::Forgot => {
                    println!("{}\n", self.text.forgot_password());
                    read_line(self.text.press_enter()).map_err(|e| e.to_string())?;
                }
                Choice::Wifi => self.wifi()?,
                Choice::Language => {
                    let next = match self.text.language {
                        Language::English => Language::Spanish,
                        Language::Spanish => Language::English,
                    };
                    self.text = Text::new(next);
                }
                Choice::Restart => {
                    // Back to OxyOS, with its tries given back, unless an erase is still to finish.
                    if self.layout.is_some() && matches!(request::read(&self.esp), Ok(Found::None))
                    {
                        let _ = Entries::new(&self.esp).rearm_all(TRIES);
                    }
                    println!("{}", self.text.restarting());
                    return crate::systemctl(&mut self.host, "reboot");
                }
                Choice::ShutDown => {
                    println!("{}", self.text.shutting_down());
                    return crate::systemctl(&mut self.host, "poweroff");
                }
            }
            println!();
        }
    }

    /// True when the disk was erased and the computer is restarting.
    fn erase_and_reinstall(&mut self) -> Result<bool, String> {
        let Some(layout) = self.layout.clone() else {
            println!("{}", self.text.no_layout());
            return Ok(false);
        };
        if layout.kind == Kind::Converted {
            println!("{}", self.text.converted_erase());
            return Ok(false);
        }
        println!("{}\n", self.text.erase_warning());
        let typed = read_line(&self.text.type_to_confirm()).map_err(|e| e.to_string())?;
        if typed.trim() != self.text.confirm_word() {
            println!("{}", self.text.cancelled());
            return Ok(false);
        }
        println!();
        let mut request = match EraseRequest::arm(&layout, crate::now()) {
            Ok(request) => request,
            Err(error) => {
                println!("{} {error}", self.text.failed());
                return Ok(false);
            }
        };
        // A root already destroyed (an erase cut short before it was asked for) reads as no file
        // system: it is made again encrypted, as an install is by default.
        request.encrypted = layout.root.encrypted() || layout.root.fstype.is_none();
        Ok(self.erase(&layout, &mut request))
    }

    /// The backups' library over the recovery's disks and Debian's restic.
    fn restorer() -> Result<Restorer, String> {
        std::fs::create_dir_all(BACKUP_CACHE).map_err(|e| format!("{BACKUP_CACHE}: {e}"))?;
        let disks = backup_disks::System::open(Path::new(BACKUP_VOLUMES))
            .map_err(|e| format!("{BACKUP_VOLUMES}: {e}"))?;
        Ok(Restorer::new(
            Engine::new(Path::new(BACKUP_CACHE)),
            Arc::new(disks) as Arc<dyn Disks>,
        ))
    }

    /// Restore from Backup: where, the password, the computer and the backup, the confirmation
    /// word; then erased, reinstalled and restored. True when it is done and the computer
    /// restarts.
    fn restore_from_backup(&mut self) -> Result<bool, String> {
        let Some(layout) = self.layout.clone().filter(|l| l.kind == Kind::Installed) else {
            println!("{}", self.text.no_layout());
            return Ok(false);
        };
        let restorer = Self::restorer()?;
        let Some(chosen) = self.choose_backup(&restorer)? else {
            return Ok(false);
        };
        let date = date(chosen.restore.time);
        println!(
            "{}\n",
            self.text.restore_warning(&chosen.restore.computer, &date)
        );
        let typed = read_line(&self.text.type_to_confirm()).map_err(|e| e.to_string())?;
        if typed.trim() != self.text.confirm_word() {
            println!("{}", self.text.cancelled());
            return Ok(false);
        }
        println!();
        let mut request = match EraseRequest::arm(&layout, crate::now()) {
            Ok(request) => request,
            Err(error) => {
                println!("{} {error}", self.text.failed());
                return Ok(false);
            }
        };
        request.encrypted = layout.root.encrypted() || layout.root.fstype.is_none();
        request.restore = Some(chosen.restore.clone());
        Ok(self.erase_and_restore(&layout, &mut request, Some((&restorer, &chosen))))
    }

    /// Where the backups are, the password, the computer and the backup; none when the person
    /// went back.
    fn choose_backup(&mut self, restorer: &Restorer) -> Result<Option<Chosen>, String> {
        loop {
            let Some((source, share_password)) = self.choose_source(restorer)? else {
                return Ok(None);
            };
            let password = read_secret(self.text.backup_password()).map_err(|e| e.to_string())?;
            println!("{}", self.text.looking());
            let opened = restorer
                .open(&source, share_password.as_deref().map(String::as_str))
                .and_then(|folder| {
                    let listing = restorer.computers(&folder, &password)?;
                    match (listing.computers.is_empty(), listing.unopened) {
                        (true, 0) => Err(Refusal::NoBackup),
                        (true, _) => Err(Refusal::WrongPassword),
                        _ => Ok((folder, listing)),
                    }
                });
            let (folder, listing) = match opened {
                Ok(opened) => opened,
                Err(refusal) => {
                    eprintln!("oxyos-recovery: the backups: {refusal}");
                    println!("{}\n", self.text.refusal(refusal.reason()));
                    continue;
                }
            };
            let computer = if let [only] = &listing.computers[..] {
                only
            } else {
                println!("\n{}", self.text.computers_heading());
                for (number, computer) in listing.computers.iter().enumerate() {
                    let newest = computer.backups.first().map_or(0, |b| b.time);
                    println!("  {:>2}  {}  ({})", number + 1, computer.name, date(newest));
                }
                let typed = read_line(self.text.choose_computer()).map_err(|e| e.to_string())?;
                match typed
                    .trim()
                    .parse::<usize>()
                    .ok()
                    .and_then(|n| listing.computers.get(n.wrapping_sub(1)))
                {
                    Some(computer) => computer,
                    None => continue,
                }
            };
            println!("\n{}", self.text.backups_heading(&computer.name));
            for (number, backup) in computer.backups.iter().enumerate() {
                println!("  {:>2}  {}", number + 1, date(backup.time));
            }
            let typed = read_line(self.text.choose_backup()).map_err(|e| e.to_string())?;
            let Some(backup) = typed
                .trim()
                .parse::<usize>()
                .ok()
                .and_then(|n| computer.backups.get(n.wrapping_sub(1)))
            else {
                continue;
            };
            let Some(repository) = folder.repository(&computer.repository, &password) else {
                continue;
            };
            // Its own accounts, read from it: the backup is checked before anything is erased.
            match restorer.manifest(&repository, &backup.snapshot) {
                Ok(manifest) => {
                    let names: Vec<&str> =
                        manifest.accounts.iter().map(|a| a.name.as_str()).collect();
                    println!("\n{}\n", self.text.accounts(&names.join(", ")));
                }
                Err(refusal) => {
                    eprintln!("oxyos-recovery: the backup: {refusal}");
                    println!("{}\n", self.text.refusal(refusal.reason()));
                    continue;
                }
            }
            let source = serde_json::to_value(&source).map_err(|e| e.to_string())?;
            return Ok(Some(Chosen {
                restore: Restore {
                    source,
                    repository: computer.repository.clone(),
                    snapshot: backup.snapshot.clone(),
                    computer: computer.name.clone(),
                    time: backup.time,
                },
                repository,
                _folder: folder,
            }));
        }
    }

    /// A disk connected, or a share and its password; none when the person went back.
    #[allow(clippy::type_complexity)]
    fn choose_source(
        &mut self,
        restorer: &Restorer,
    ) -> Result<Option<(Source, Option<Zeroizing<String>>)>, String> {
        loop {
            println!("{}\n", self.text.restore_where());
            let disks = restorer.disks();
            for (number, disk) in disks.iter().enumerate() {
                let name = if disk.label.is_empty() {
                    format!("{:.1} GB", disk.size as f64 / 1e9)
                } else {
                    format!("{} ({:.1} GB)", disk.label, disk.size as f64 / 1e9)
                };
                println!("  {:>2}  {name}", number + 1);
            }
            println!("  {:>2}  {}", disks.len() + 1, self.text.network_share());
            println!("  {:>2}  {}", disks.len() + 2, self.text.look_again());
            println!();
            let typed = read_line(self.text.choose_or_back()).map_err(|e| e.to_string())?;
            let Ok(number) = typed.trim().parse::<usize>() else {
                if typed.trim().is_empty() {
                    return Ok(None);
                }
                continue;
            };
            if let Some(disk) = disks.get(number.wrapping_sub(1)) {
                return Ok(Some((
                    Source::Disk {
                        uuid: disk.uuid.clone(),
                    },
                    None,
                )));
            }
            if number == disks.len() + 1 {
                let share = Share {
                    host: read_line(self.text.share_host())
                        .map_err(|e| e.to_string())?
                        .trim()
                        .into(),
                    port: None,
                    share: read_line(self.text.share_name())
                        .map_err(|e| e.to_string())?
                        .trim()
                        .into(),
                    user: read_line(self.text.share_user())
                        .map_err(|e| e.to_string())?
                        .trim()
                        .into(),
                    domain: None,
                };
                if share.validate().is_err() {
                    println!("{}\n", self.text.refusal("unreachable"));
                    continue;
                }
                let password =
                    read_secret(self.text.share_password()).map_err(|e| e.to_string())?;
                return Ok(Some((Source::Share { share }, Some(password))));
            }
        }
    }

    /// A restore a power cut interrupted: its backup opened again with its password, then the
    /// erase resumed and the backup restored. The person may finish the erase without it.
    fn resume_restore(
        &mut self,
        layout: &Layout,
        request: &mut EraseRequest,
        restore: &Restore,
    ) -> bool {
        let date = date(restore.time);
        println!("{}\n", self.text.interrupted(&restore.computer, &date));
        let restorer = match Self::restorer() {
            Ok(restorer) => restorer,
            Err(error) => {
                println!("{} {error}", self.text.failed());
                return false;
            }
        };
        loop {
            match self.reopen(&restorer, restore) {
                Ok(Some(chosen)) => {
                    println!("{}\n", self.text.pending_erase());
                    return self.erase_and_restore(layout, request, Some((&restorer, &chosen)));
                }
                Ok(None) => {}
                Err(error) => {
                    println!("{} {error}", self.text.failed());
                    return false;
                }
            }
            for (number, item) in self.text.interrupted_menu().iter().enumerate() {
                println!("  {}  {item}", number + 1);
            }
            let typed = match read_line(self.text.choose()) {
                Ok(typed) => typed,
                Err(_) => return false,
            };
            println!();
            match typed.trim() {
                "2" => {
                    if self.wifi().is_err() {
                        return false;
                    }
                }
                "3" => {
                    request.restore = None;
                    println!("{}\n", self.text.pending_erase());
                    return self.erase(layout, request);
                }
                _ => {}
            }
        }
    }

    /// The interrupted restore's backup, opened again; none (said why) when it cannot be.
    fn reopen(&mut self, restorer: &Restorer, restore: &Restore) -> Result<Option<Chosen>, String> {
        let source: Source = match serde_json::from_value(restore.source.clone()) {
            Ok(source) => source,
            Err(_) => {
                println!("{}\n", self.text.refusal("invalidBackup"));
                return Ok(None);
            }
        };
        let share_password = match &source {
            Source::Share { .. } => {
                Some(read_secret(self.text.share_password()).map_err(|e| e.to_string())?)
            }
            Source::Disk { .. } => None,
        };
        let password = read_secret(self.text.backup_password()).map_err(|e| e.to_string())?;
        let opened = restorer
            .open(&source, share_password.as_deref().map(String::as_str))
            .and_then(|folder| {
                let repository = folder
                    .repository(&restore.repository, &password)
                    .ok_or(Refusal::NoBackup)?;
                restorer.manifest(&repository, &restore.snapshot)?;
                Ok((folder, repository))
            });
        match opened {
            Ok((folder, repository)) => Ok(Some(Chosen {
                _folder: folder,
                repository,
                restore: restore.clone(),
            })),
            Err(refusal) => {
                eprintln!("oxyos-recovery: the backup: {refusal}");
                println!("{}\n", self.text.refusal(refusal.reason()));
                Ok(None)
            }
        }
    }

    fn check_disk(&mut self) -> Result<(), String> {
        let Some(layout) = self.layout.clone() else {
            println!("{}", self.text.no_layout());
            return Ok(());
        };
        let secret = if layout.root.encrypted() {
            Some(read_secret(self.text.secret_prompt()).map_err(|e| e.to_string())?)
        } else {
            None
        };
        println!("{}", self.text.checking());
        match check::check(
            &mut self.host,
            &layout,
            secret.as_ref().map(|s| s.as_bytes()),
        ) {
            Ok(report) => {
                println!(
                    "  {}: {}",
                    self.text.esp_name(),
                    self.text.checked(report.esp)
                );
                println!(
                    "  {}: {}",
                    self.text.system_name(),
                    self.text.checked(report.root)
                );
            }
            Err(Refused::WrongSecret) => println!("{}", self.text.wrong_secret()),
            Err(Refused::Failed(error)) => println!("{} {error}", self.text.failed()),
        }
        Ok(())
    }

    fn wifi(&mut self) -> Result<(), String> {
        println!("{}\n", self.text.online(network::online(&mut self.host)));
        let networks = network::networks(&mut self.host).unwrap_or_default();
        if networks.is_empty() {
            println!("{}", self.text.no_networks());
            return Ok(());
        }
        println!("{}", self.text.networks_heading());
        for (number, network) in networks.iter().enumerate() {
            let secured = if network.secured {
                self.text.secured()
            } else {
                ""
            };
            println!(
                "  {:>2}  {}  {}%{secured}",
                number + 1,
                network.ssid,
                network.signal
            );
        }
        let choice = read_line(self.text.choose_network()).map_err(|e| e.to_string())?;
        let Some(network) = choice
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|n| networks.get(n.wrapping_sub(1)))
        else {
            return Ok(());
        };
        let password = if network.secured {
            Some(read_secret(self.text.network_password()).map_err(|e| e.to_string())?)
        } else {
            None
        };
        match network::join(
            &mut self.host,
            &network.ssid,
            password.as_deref().map(String::as_str),
        ) {
            Ok(()) => println!("{}", self.text.joined()),
            Err(_) => println!("{}", self.text.join_failed()),
        }
        Ok(())
    }
}

/// A backup's time as the person reads it: the recovery's clock is the firmware's, in UTC.
fn date(seconds: i64) -> String {
    jiff::Timestamp::from_second(seconds)
        .map(|t| t.strftime("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|_| seconds.to_string())
}
