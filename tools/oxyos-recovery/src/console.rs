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

use crate::messages::{Language, Text};
use crate::terminal::{read_line, read_secret};
use oxyos_recovery::check::{self, Refused};
use oxyos_recovery::disks::{self, Kind, Layout};
use oxyos_recovery::entries::{Entries, TRIES};
use oxyos_recovery::host::{Command, Host, System};
use oxyos_recovery::install::{self, Paths};
use oxyos_recovery::layout::Image;
use oxyos_recovery::network;
use oxyos_recovery::request::{self, EraseRequest, Found, Verdict};
use std::path::PathBuf;

/// Where the recovery mounts its disk's EFI system partition.
const ESP: &str = "/run/oxyos-recovery/esp";
/// What live-boot mounted the recovery from.
const MEDIUM: &str = "/run/live/medium";

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
        // A converted machine's EFI system partition is its own GRUB's: read, never written.
        let mut mount = Command::new("mount");
        if layout.kind == Kind::Converted {
            mount = mount.args(["-o", "ro"]);
        }
        let mounted = layout.esp.mounted_at(ESP)
            || self.host.run(&mount.args([&layout.esp.path, ESP])).is_ok();
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
        println!("{}\n", self.text.pending_erase());
        self.erase(&layout, &mut request)
    }

    /// Erases and installs again; true when it is done and the computer restarts.
    fn erase(&mut self, layout: &Layout, request: &mut EraseRequest) -> bool {
        let text = self.text;
        let result = install::erase(
            &mut self.host,
            &Paths::default(),
            &Image::live_boot(),
            &self.esp,
            layout,
            request,
            &mut |step| {
                println!("{}", text.step(step));
            },
        );
        match result {
            Ok(()) => {
                println!("\n{}", text.erased());
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

    fn menu(&mut self) -> Result<(), String> {
        loop {
            println!("{}\n", self.text.title());
            for (number, item) in self.text.menu().iter().enumerate() {
                println!("  {}  {item}", number + 1);
            }
            println!();
            let choice = read_line(self.text.choose()).map_err(|e| e.to_string())?;
            println!();
            match choice.trim() {
                "1" => {
                    if self.erase_and_reinstall()? {
                        return Ok(());
                    }
                }
                "2" => self.check_disk()?,
                "3" => {
                    println!("{}\n", self.text.forgot_password());
                    read_line(self.text.press_enter()).map_err(|e| e.to_string())?;
                }
                "4" => self.wifi()?,
                "5" => {
                    let next = match self.text.language {
                        Language::English => Language::Spanish,
                        Language::Spanish => Language::English,
                    };
                    self.text = Text::new(next);
                }
                "6" => {
                    // Back to OxyOS, with its tries given back, unless an erase is still to finish.
                    // A converted machine's GRUB gives its tries back by itself.
                    if self
                        .layout
                        .as_ref()
                        .is_some_and(|l| l.kind == Kind::Installed)
                        && matches!(request::read(&self.esp), Ok(Found::None))
                    {
                        let _ = Entries::new(&self.esp).rearm_all(TRIES);
                    }
                    println!("{}", self.text.restarting());
                    return crate::systemctl(&mut self.host, "reboot");
                }
                "7" => {
                    println!("{}", self.text.shutting_down());
                    return crate::systemctl(&mut self.host, "poweroff");
                }
                _ => {}
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
