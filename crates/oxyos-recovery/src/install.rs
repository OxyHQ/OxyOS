//! Installing ([ADR 0023](../../../docs/adr/0023-installing-oxyos.mdx), decision 1) and erasing
//! ([ADR 0054](../../../docs/adr/0054-recovery-and-erasing.mdx), decision 4): the tested image written
//! whole, never a system assembled on the disk.
//!
//! **Installing** (from the live medium, onto a whole disk):
//! 1. the medium's image is checked against its root hash (`veritysetup verify`): what is written
//!    is the image the acceptance tests booted and the audit read, every block of it;
//! 2. systemd-repart lays the disk out and fills it ([`layout::definitions`]): the EFI system
//!    partition, OxyOS Recovery with the medium's `/live/`, the root with the image's tree,
//!    encrypted with a new install key;
//! 3. the recovery's copy is checked against the same root hash;
//! 4. the root is set up as an installed system ([`set_up`]), and the recovery's entry is written.
//!
//! **Erasing** (from the recovery, keeping the EFI system partition and the recovery):
//! 1. every OxyOS entry loses its tries and the request says `erasing`, so nothing but the recovery
//!    starts until the erase is done, even across a power cut;
//! 2. the root is destroyed: a LUKS2 root's keyslots are erased (`cryptsetup erase`: the volume key
//!    is gone, and with it every byte the volume ever held) and its header wiped; then the whole
//!    partition is discarded. A root that was never encrypted can only be discarded or overwritten;
//! 3. the old kernels' entries go, and systemd-repart makes the root again (`--factory-reset=yes`)
//!    from the recovery's own image, with a new install key;
//! 4. the root is set up as an installed system, exactly as an install sets it up; the request is
//!    removed. The next boot is a first boot.

use crate::disks::{self, Disk, Kind, Layout};
use crate::entries::{Entries, TRIES};
use crate::host::{failed, Command, Failed, Host};
use crate::layout::{self, Image};
use crate::request::{self, EraseRequest, State};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// Where the work is done.
#[derive(Clone, Debug)]
pub struct Paths {
    /// A directory of the program's own on a RAM file system (`/run/oxyos-recovery`): the
    /// definitions, the install key, the mount points.
    pub run: PathBuf,
    /// efivarfs (`/sys/firmware/efi/efivars`): an install is UEFI's (ADR 0023, decision 4).
    pub efivars: PathBuf,
}

impl Default for Paths {
    fn default() -> Self {
        Self {
            run: "/run/oxyos-recovery".into(),
            efivars: "/sys/firmware/efi/efivars".into(),
        }
    }
}

impl Paths {
    fn target(&self) -> PathBuf {
        self.run.join("target")
    }
    fn definitions(&self) -> PathBuf {
        self.run.join("repart.d")
    }
    fn key(&self) -> PathBuf {
        self.run.join("root.key")
    }
    fn recovery(&self) -> PathBuf {
        self.run.join("recovery")
    }
}

/// A development user ([ADR 0023](../../../docs/adr/0023-installing-oxyos.mdx), decision 5): a local
/// Unix user with a password of the machine, only on a development install, until the login
/// program's setup is in the image. With encryption, its password unlocks the disk instead of the
/// install key, and a recovery key is made.
pub struct Developer {
    pub user: String,
    pub password: Zeroizing<String>,
}

pub struct Options {
    pub encrypt: bool,
    pub developer: Option<Developer>,
}

/// What an install leaves to tell.
#[derive(Debug, Default)]
pub struct Installed {
    /// The disk's recovery key, when one was made: shown once, to be written down.
    pub recovery_key: Option<Zeroizing<String>>,
}

/// The steps a person is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    CheckingImage,
    Partitioning,
    CheckingRecovery,
    SettingUp,
    Erasing,
    Recreating,
    Finishing,
}

/// The disks, as lsblk sees them now.
pub fn disks(host: &mut dyn Host) -> Result<Vec<Disk>, Failed> {
    let json = host.run(&Command::new("lsblk").args(disks::LSBLK_ARGUMENTS.iter().copied()))?;
    disks::parse(&json).map_err(|e| failed("the disks", e))
}

/// The OxyOS layout on the disk at `path`, as it is now.
pub fn layout_of(host: &mut dyn Host, path: &str) -> Result<Layout, Failed> {
    disks(host)?
        .into_iter()
        .find(|d| d.path == path)
        .ok_or_else(|| failed(format!("{path} is gone"), ""))?
        .layout()
        .ok_or_else(|| failed(format!("{path} has no OxyOS layout"), ""))
}

/// The medium's root hash, the one its image is checked against.
pub fn root_hash(image: &Image) -> Result<String, Failed> {
    let path = format!("{}.roothash", image.squashfs());
    let text = fs::read_to_string(&path)
        .map_err(|e| failed(format!("the image's root hash ({path})"), e.to_string()))?;
    let hash = text.trim().to_ascii_lowercase();
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(failed("the image's root hash", "not a SHA-256 root hash"));
    }
    Ok(hash)
}

/// Checks every block of the squashfs in `live` against its hash tree and `root_hash`.
fn verify(host: &mut dyn Host, live: &str, root_hash: &str) -> Result<(), Failed> {
    let squashfs = format!("{live}/filesystem.squashfs");
    host.run(&Command::new("veritysetup").args([
        "verify",
        &squashfs,
        &format!("{squashfs}.verity"),
        root_hash,
    ]))
    .map(drop)
    .map_err(|e| failed(format!("{squashfs} does not match its root hash"), e.detail))
}

/// The kernel version of the medium's kernel (`vmlinuz-<version>` in `live`).
fn kernel_version(live: &str) -> Result<String, Failed> {
    let mut versions: Vec<String> = fs::read_dir(live)
        .map_err(|e| failed(format!("the medium's {live}"), e.to_string()))?
        .filter_map(|e| {
            e.ok()?
                .file_name()
                .to_str()?
                .strip_prefix("vmlinuz-")
                .map(String::from)
        })
        .collect();
    versions.sort();
    match versions.as_slice() {
        [version] => Ok(version.clone()),
        [] => Err(failed("the medium has no kernel", "")),
        _ => Err(failed(
            "the medium has more than one kernel",
            versions.join(", "),
        )),
    }
}

/// A new random key of 64 bytes, in a file only root can read.
fn new_key(path: &Path) -> Result<(), Failed> {
    let mut key = Zeroizing::new([0u8; 64]);
    fs::File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(&mut key[..]))
        .map_err(|e| failed("a new key", e.to_string()))?;
    let _ = fs::remove_file(path);
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .and_then(|mut file| file.write_all(&key[..]))
        .map_err(|e| failed("a new key", e.to_string()))
}

fn write_definitions(paths: &Paths, image: &Image, encrypt: bool) -> Result<(), Failed> {
    let directory = paths.definitions();
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory)
        .map_err(|e| failed("the partitions' definitions", e.to_string()))?;
    for (name, text) in layout::definitions(image, encrypt) {
        fs::write(directory.join(name), text)
            .map_err(|e| failed("the partitions' definitions", e.to_string()))?;
    }
    Ok(())
}

fn repart(paths: &Paths, disk: &str, encrypt: bool, mode: &str) -> Command {
    let mut command = Command::new("systemd-repart")
        .env("SYSTEMD_REPART_MKFS_OPTIONS_EXT4", layout::EXT4_OPTIONS)
        .arg("--dry-run=no")
        .arg(mode)
        .arg(format!("--definitions={}", paths.definitions().display()));
    if encrypt {
        command = command.arg(format!("--key-file={}", paths.key().display()));
    }
    command.arg(disk)
}

/// Installs onto the whole disk at `disk`, erasing it, from the live medium's `image`.
pub fn install(
    host: &mut dyn Host,
    paths: &Paths,
    image: &Image,
    disk: &str,
    options: &Options,
    progress: &mut dyn FnMut(Step),
) -> Result<Installed, Failed> {
    if !paths.efivars.is_dir() {
        return Err(failed(
            "this machine did not start through UEFI",
            "OxyOS installs on UEFI machines only",
        ));
    }
    fs::create_dir_all(&paths.run).map_err(|e| failed("the work directory", e.to_string()))?;
    progress(Step::CheckingImage);
    let hash = root_hash(image)?;
    verify(host, &image.live, &hash)?;
    let version = kernel_version(&image.live)?;

    progress(Step::Partitioning);
    write_definitions(paths, image, options.encrypt)?;
    if options.encrypt {
        new_key(&paths.key())?;
    }
    host.run(&repart(paths, disk, options.encrypt, "--empty=force"))?;
    host.run(&Command::new("udevadm").arg("settle"))?;
    let layout = layout_of(host, disk)?;

    progress(Step::CheckingRecovery);
    let recovery = paths.recovery();
    fs::create_dir_all(&recovery).map_err(|e| failed("a mount point", e.to_string()))?;
    host.run(&Command::new("mount").args([
        "-o",
        "ro",
        &layout.recovery.path,
        &recovery.display().to_string(),
    ]))?;
    let checked = verify(host, &format!("{}/live", recovery.display()), &hash);
    host.run(&Command::new("umount").arg(recovery.display().to_string()))?;
    checked?;

    progress(Step::SettingUp);
    let recovery_entry = RecoveryEntry {
        image,
        root_hash: &hash,
        version: &version,
    };
    set_up(host, paths, &layout, options, Some(&recovery_entry))
}

/// What the recovery's entry is made from, at install.
struct RecoveryEntry<'a> {
    image: &'a Image,
    root_hash: &'a str,
    version: &'a str,
}

/// What is mounted for the set-up, undone in reverse whatever happens.
struct Mounts<'h> {
    host: &'h mut dyn Host,
    mounted: Vec<String>,
    opened: Option<String>,
}

impl Mounts<'_> {
    fn mount(&mut self, arguments: &[&str], at: &Path) -> Result<(), Failed> {
        fs::create_dir_all(at).map_err(|e| failed(format!("{}", at.display()), e.to_string()))?;
        let at = at.display().to_string();
        self.host.run(
            &Command::new("mount")
                .args(arguments.iter().copied())
                .arg(&at),
        )?;
        self.mounted.push(at);
        Ok(())
    }

    fn undo(&mut self) -> Result<(), Failed> {
        let mut first = None;
        while let Some(at) = self.mounted.pop() {
            if let Err(e) = self.host.run(&Command::new("umount").arg(&at)) {
                first.get_or_insert(e);
            }
        }
        if let Some(name) = self.opened.take() {
            if let Err(e) = self
                .host
                .run(&Command::new("cryptsetup").args(["close", &name]))
            {
                first.get_or_insert(e);
            }
        }
        first.map_or(Ok(()), Err)
    }
}

/// The live system's own packages, which an installed system has no use for; purging `live-tools`
/// gives back `update-initramfs`, which it diverts to its own. GRUB, the live medium's boot loader,
/// goes too ([`live_boot_loader`]): an installed system boots with systemd-boot, whose maintainer
/// script installs shim only where no signed GRUB claims the EFI system partition.
pub const LIVE_PACKAGES: &[&str] = &[
    "live-boot",
    "live-boot-initramfs-tools",
    "live-boot-doc",
    "live-config",
    "live-config-systemd",
    "live-config-doc",
    "live-tools",
    "user-setup",
];

/// The live medium's boot loader among `installed` (`dpkg-query`'s package names): GRUB's.
pub fn live_boot_loader(installed: &str) -> Vec<String> {
    installed
        .lines()
        .map(str::trim)
        .filter(|name| name.starts_with("grub-") || name.starts_with("grub2-"))
        .map(String::from)
        .collect()
}

/// The root on `layout` set up as an installed system: opened and mounted with the EFI system
/// partition at `/efi`, the live packages purged, its tables written, systemd-boot and shim
/// installed by Debian's own maintainer script, its initramfs made (kernel-install writes each
/// kernel's entry, counted), the development user made when asked. Everything mounted is unmounted
/// and closed, whether it succeeds or not.
fn set_up(
    host: &mut dyn Host,
    paths: &Paths,
    layout: &Layout,
    options: &Options,
    recovery_entry: Option<&RecoveryEntry<'_>>,
) -> Result<Installed, Failed> {
    let mut mounts = Mounts {
        host,
        mounted: Vec::new(),
        opened: None,
    };
    let result = set_up_mounted(&mut mounts, paths, layout, options, recovery_entry);
    let undone = mounts.undo();
    let installed = result?;
    undone?;
    Ok(installed)
}

fn set_up_mounted(
    mounts: &mut Mounts<'_>,
    paths: &Paths,
    layout: &Layout,
    options: &Options,
    recovery_entry: Option<&RecoveryEntry<'_>>,
) -> Result<Installed, Failed> {
    let target = paths.target();
    let key = paths.key();
    let key_text = key.display().to_string();
    let encrypted = layout.root.encrypted();
    if encrypted != options.encrypt {
        return Err(failed("the root's encryption is not what was asked", ""));
    }
    let luks_uuid = layout
        .root
        .uuid
        .clone()
        .ok_or_else(|| failed("the root has no UUID", ""))?;
    let device = if encrypted {
        // Opened under the name its crypttab gives it: cryptsetup's initramfs hook finds the root's
        // entry by the mapping the target's / is mounted from.
        let name = layout::ROOT_MAPPER;
        mounts.host.run(&Command::new("cryptsetup").args([
            "open",
            "--key-file",
            &key_text,
            &layout.root.path,
            name,
        ]))?;
        mounts.opened = Some(name.into());
        format!("/dev/mapper/{name}")
    } else {
        layout.root.path.clone()
    };
    mounts.mount(&[&device], &target)?;
    mounts.mount(&[&layout.esp.path], &target.join("efi"))?;
    for (source, kind) in [
        ("/dev", "--bind"),
        ("/dev/pts", "--bind"),
        ("/proc", "--bind"),
        ("/sys", "--bind"),
        ("/run", "--bind"),
    ] {
        mounts.mount(&[kind, source], &target.join(&source[1..]))?;
    }
    mounts.mount(
        &["--bind", &paths.efivars.display().to_string()],
        &target.join("sys/firmware/efi/efivars"),
    )?;

    let root_fs_uuid = blkid(mounts.host, &device)?;
    let esp_fs_uuid = layout
        .esp
        .uuid
        .clone()
        .ok_or_else(|| failed("the EFI system partition has no UUID", ""))?;
    let chroot = |command: &str| {
        Command::new("chroot")
            .arg(target.display().to_string())
            .args(command.split(' '))
            .env("DEBIAN_FRONTEND", "noninteractive")
            .env("LC_ALL", "C.UTF-8")
    };

    let installed = mounts
        .host
        .run(&chroot("dpkg-query --show --showformat=${Package}\\n"))?;
    // GRUB's signed package is protected: purging it is said in so many words.
    mounts.host.run(
        &chroot("apt-get purge --yes --quiet --allow-remove-essential")
            .args(LIVE_PACKAGES.iter().copied())
            .args(live_boot_loader(&installed)),
    )?;

    let write = |relative: &str, text: &str, mode: u32| -> Result<(), Failed> {
        let path = target.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| failed(relative.to_string(), e.to_string()))?;
        }
        fs::write(&path, text)
            .and_then(|()| fs::set_permissions(&path, fs::Permissions::from_mode(mode)))
            .map_err(|e| failed(relative.to_string(), e.to_string()))
    };
    let root_device = if encrypted {
        format!("/dev/mapper/{}", layout::ROOT_MAPPER)
    } else {
        format!("UUID={root_fs_uuid}")
    };
    write(
        "etc/fstab",
        &layout::fstab(&root_fs_uuid, &esp_fs_uuid),
        0o644,
    )?;
    write(
        "etc/kernel/cmdline",
        &format!("{}\n", layout::kernel_command_line(&root_device)),
        0o644,
    )?;
    write("etc/kernel/tries", &format!("{TRIES}\n"), 0o644)?;
    write("etc/kernel/entry-token", "oxyos\n", 0o644)?;
    write("etc/hostname", "oxyos\n", 0o644)?;
    write(
        "etc/hosts",
        "127.0.0.1\tlocalhost\n127.0.1.1\toxyos\n::1\tlocalhost ip6-localhost ip6-loopback\n",
        0o644,
    )?;
    // No machine id: systemd makes one at the first boot and runs the first boot's units.
    write("etc/machine-id", "", 0o444)?;
    let _ = fs::remove_file(target.join("var/lib/dbus/machine-id"));

    let mut installed = Installed::default();
    if encrypted {
        let developer_unlocks = options.developer.is_some();
        if let Some(developer) = &options.developer {
            // The developer's password becomes the disk's, a recovery key is made, and the install
            // key leaves the disk, as setup does with the account's (ADR 0023, decision 3).
            mounts.host.run(
                &Command::new("cryptsetup")
                    .args([
                        "luksAddKey",
                        "--batch-mode",
                        "--key-file",
                        &key_text,
                        &layout.root.path,
                        "-",
                    ])
                    .input(developer.password.as_bytes()),
            )?;
            let said = mounts.host.run(&Command::new("systemd-cryptenroll").args([
                &format!("--unlock-key-file={key_text}"),
                "--recovery-key",
                &layout.root.path,
            ]))?;
            installed.recovery_key = said
                .split_whitespace()
                .find(|w| is_recovery_key(w))
                .map(|w| Zeroizing::new(w.to_string()));
            if installed.recovery_key.is_none() {
                return Err(failed("systemd-cryptenroll made no recovery key", ""));
            }
            mounts.host.run(&Command::new("cryptsetup").args([
                "luksRemoveKey",
                "--batch-mode",
                &layout.root.path,
                &key_text,
            ]))?;
        }
        let key_in_target = (!developer_unlocks).then_some(Path::new(layout::INSTALL_KEY));
        write(
            "etc/crypttab",
            &layout::crypttab(&luks_uuid, key_in_target),
            0o644,
        )?;
        if !developer_unlocks {
            let copy = target.join(&layout::INSTALL_KEY[1..]);
            fs::create_dir_all(copy.parent().expect("in a directory"))
                .map_err(|e| failed("the install key", e.to_string()))?;
            fs::set_permissions(
                copy.parent().expect("in a directory"),
                fs::Permissions::from_mode(0o700),
            )
            .map_err(|e| failed("the install key", e.to_string()))?;
            fs::copy(&key, &copy)
                .and_then(|_| fs::set_permissions(&copy, fs::Permissions::from_mode(0o600)))
                .map_err(|e| failed("the install key", e.to_string()))?;
        }
    }

    if let Some(developer) = &options.developer {
        mounts.host.run(
            &chroot("useradd --create-home --user-group --groups sudo --shell /bin/bash")
                .arg(&developer.user),
        )?;
        let mut line = Zeroizing::new(format!("{}:", developer.user));
        line.push_str(&developer.password);
        line.push('\n');
        mounts
            .host
            .run(&chroot("chpasswd").input(line.as_bytes()))?;
    }

    // systemd-boot and shim onto the EFI system partition, and the firmware's entry for shim, as
    // Debian's systemd-boot package does when it finds an EFI system partition.
    mounts.host.run(&chroot(
        "dpkg-reconfigure --frontend=noninteractive systemd-boot",
    ))?;
    write("efi/loader/loader.conf", layout::LOADER_CONF, 0o644)?;

    if let Some(entry) = recovery_entry {
        let directory = target.join("efi").join(layout::RECOVERY_DIRECTORY);
        fs::create_dir_all(&directory)
            .map_err(|e| failed("the recovery's kernel", e.to_string()))?;
        for (from, to) in [("vmlinuz", "linux"), ("initrd.img", "initrd.img")] {
            fs::copy(
                format!("{}/{from}-{}", entry.image.live, entry.version),
                directory.join(to),
            )
            .map_err(|e| failed(format!("the recovery's {to}"), e.to_string()))?;
        }
        let partuuid = layout
            .recovery
            .partuuid
            .as_deref()
            .ok_or_else(|| failed("the recovery has no partition UUID", ""))?;
        write(
            &format!("efi/loader/entries/{}", layout::RECOVERY_ENTRY),
            &layout::recovery_entry(partuuid, entry.root_hash, entry.version),
            0o644,
        )?;
    }

    // The initramfs with the unlock and the boot logo; Debian's systemd-boot hook has
    // kernel-install write each kernel's entry, given /etc/kernel/tries to start with.
    mounts.host.run(&chroot("update-initramfs -u -k all"))?;
    let entries = Entries::new(&target.join("efi"));
    if entries
        .system()
        .map_err(|e| failed("the boot entries", e.to_string()))?
        .is_empty()
    {
        return Err(failed("no boot entry was written for OxyOS's kernel", ""));
    }
    mounts.host.run(&Command::new("sync"))?;
    Ok(installed)
}

/// A recovery key as systemd-cryptenroll prints it: eight groups of eight modhex letters.
fn is_recovery_key(word: &str) -> bool {
    let groups: Vec<&str> = word.split('-').collect();
    groups.len() == 8
        && groups
            .iter()
            .all(|g| g.len() == 8 && g.bytes().all(|b| b"cbdefghijklnrtuv".contains(&b)))
}

fn blkid(host: &mut dyn Host, device: &str) -> Result<String, Failed> {
    let uuid = host.run(&Command::new("blkid").args([
        "--match-tag",
        "UUID",
        "--output",
        "value",
        device,
    ]))?;
    let uuid = uuid.trim();
    if uuid.is_empty() {
        return Err(failed(format!("{device} has no UUID"), ""));
    }
    Ok(uuid.to_string())
}

/// Erases the root of `layout` and makes it again from the recovery's `image`, following
/// `request` (rewritten `erasing` first). `esp` is where the recovery mounted the EFI system
/// partition, read-write.
pub fn erase(
    host: &mut dyn Host,
    paths: &Paths,
    image: &Image,
    esp: &Path,
    layout: &Layout,
    request: &mut EraseRequest,
    progress: &mut dyn FnMut(Step),
) -> Result<(), Failed> {
    // A converted machine's disk is not an install's: systemd-repart's definitions do not describe
    // it, and nothing here erases it.
    if layout.kind != Kind::Installed {
        return Err(failed(
            "this computer was converted to OxyOS by hand",
            "erasing it is installing OxyOS onto it from the installation medium",
        ));
    }
    fs::create_dir_all(&paths.run).map_err(|e| failed("the work directory", e.to_string()))?;
    progress(Step::Erasing);
    let entries = Entries::new(esp);
    entries
        .condemn_all()
        .map_err(|e| failed("the boot entries", e.to_string()))?;
    request.state = State::Erasing;
    request::write(esp, request).map_err(|e| failed("the erase request", e.to_string()))?;

    destroy(host, &layout.root)?;

    progress(Step::Recreating);
    // The old system's kernels go with it: the image brings its own.
    entries
        .remove_system()
        .map_err(|e| failed("the boot entries", e.to_string()))?;
    match fs::remove_dir_all(esp.join("oxyos")) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(failed("the old kernels", e.to_string())),
    }
    // The image is the one the recovery runs from, every block read through dm-verity against the
    // root hash on its entry's command line.
    write_definitions(paths, image, request.encrypted)?;
    if request.encrypted {
        new_key(&paths.key())?;
    }
    host.run(&repart(
        paths,
        &layout.disk.path,
        request.encrypted,
        "--factory-reset=yes",
    ))?;
    host.run(&Command::new("udevadm").arg("settle"))?;
    let layout = layout_of(host, &layout.disk.path)?;

    progress(Step::SettingUp);
    let options = Options {
        encrypt: request.encrypted,
        developer: None,
    };
    set_up(host, paths, &layout, &options, None)?;

    progress(Step::Finishing);
    request::remove(esp).map_err(|e| failed("the erase request", e.to_string()))?;
    let _ = fs::remove_file(paths.key());
    Ok(())
}

/// Destroys what the root partition holds.
///
/// - LUKS2: `cryptsetup erase` overwrites every keyslot, so the volume key, and every byte the
///   volume ever held, can no longer be read by anyone (a crypto-erase, what Apple's effaceable
///   storage does); `wipefs` removes the header.
/// - Then the partition is discarded: flash drops what it held. A root that was never encrypted
///   gets a secure discard where the disk offers one, else a discard, else zeroes written over it,
///   which on flash cannot reach every cell: that is why encryption is on by default.
pub fn destroy(host: &mut dyn Host, root: &disks::Partition) -> Result<(), Failed> {
    if root.encrypted() {
        host.run(&Command::new("cryptsetup").args(["erase", "--batch-mode", &root.path]))?;
        host.run(&Command::new("wipefs").args(["--all", &root.path]))?;
        // Best effort: the volume key is gone already.
        let _ = host.execute(&Command::new("blkdiscard").args(["--force", &root.path]));
        return Ok(());
    }
    host.run(&Command::new("wipefs").args(["--all", &root.path]))?;
    for flags in [
        &["--force", "--secure"][..],
        &["--force"],
        &["--force", "--zeroout"],
    ] {
        let command = Command::new("blkdiscard")
            .args(flags.iter().copied())
            .arg(&root.path);
        if host.execute(&command)?.status == 0 {
            return Ok(());
        }
    }
    Err(failed(
        format!("{} could not be erased", root.path),
        "neither discarded nor overwritten",
    ))
}
