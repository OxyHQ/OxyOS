//! The disk an install writes ([ADR 0023](../../../docs/adr/0023-installing-oxyos.mdx), decision 2,
//! with the recovery of [ADR 0054](../../../docs/adr/0054-recovery-and-erasing.mdx), decision 1): the
//! whole disk, GPT, three partitions, each known by its type (the Discoverable Partitions
//! Specification's for the EFI system partition and the root, OxyOS's own for the recovery):
//!
//! | Partition | Type | Size | Contents |
//! | --- | --- | --- | --- |
//! | `ESP` | EFI system partition | 1 GiB | shim, systemd-boot, the entries, each kernel and initramfs, the recovery's kernel and initramfs, the erase request |
//! | `OxyOS Recovery` | [`RECOVERY_TYPE`] | 3 GiB | the image the machine was installed from: `/live/` of the live medium, read-only |
//! | `OxyOS` | the architecture's root | the rest | the system and everything of its people, inside LUKS2 when encrypted |
//!
//! systemd-repart lays it out from [`definitions`]. The root carries `FactoryReset=yes`: erasing
//! (`--factory-reset=yes`) deletes it and makes it again from the recovery's image, and nothing
//! else on the disk is touched.

use std::fmt::Write as _;
use std::path::Path;

/// The EFI system partition's type.
pub const ESP_TYPE: &str = "c12a7328-f81f-11d2-ba4b-00a0c93ec93b";
/// OxyOS Recovery's type: OxyOS's own, so no other system's tools take it for theirs (systemd's
/// discovery mounts no partition of a type it does not know), and nothing confuses it with a
/// partition of data.
pub const RECOVERY_TYPE: &str = "ec68fd51-3b26-41e5-a32e-b0dff6fc5484";
/// The root's type on x86-64 (the Discoverable Partitions Specification's).
pub const ROOT_TYPE_X86_64: &str = "4f68bce3-e8cd-4db1-96e7-fbcaf984b709";
/// The root's type on AArch64.
pub const ROOT_TYPE_ARM64: &str = "b921b045-1df0-41c3-af44-4c6f280d3fae";

/// The partitions' labels, as `lsblk` and the firmware show them.
pub const ESP_LABEL: &str = "ESP";
pub const RECOVERY_LABEL: &str = "OxyOS Recovery";
pub const ROOT_LABEL: &str = "OxyOS";

const GIB: u64 = 1 << 30;
/// The EFI system partition: several kernels with their initramfs images (about 140 MiB each) and
/// the recovery's (about 140 MiB).
pub const ESP_SIZE: u64 = GIB;
/// OxyOS Recovery: the image (a 0.8 GiB squashfs with its hash tree today, the session and runtime
/// to come) with room for the next one.
pub const RECOVERY_SIZE: u64 = 3 * GIB;
/// The least the root may have.
pub const ROOT_MIN_SIZE: u64 = 8 * GIB;
/// The least a disk must have to take an install: the three partitions and GPT's own sectors.
pub const MIN_DISK_SIZE: u64 = ESP_SIZE + RECOVERY_SIZE + ROOT_MIN_SIZE + 2 * (1 << 20);

/// The name the root's LUKS2 volume is opened under, on the installed system and while
/// installing.
pub const ROOT_MAPPER: &str = "oxyos-root";

/// The root's type on the machine this runs on.
pub fn root_type() -> &'static str {
    root_type_for(std::env::consts::ARCH)
}

/// The root's type on `architecture` (Rust's names: `x86_64`, `aarch64`).
pub fn root_type_for(architecture: &str) -> &'static str {
    match architecture {
        "aarch64" => ROOT_TYPE_ARM64,
        _ => ROOT_TYPE_X86_64,
    }
}

/// Where an image to install comes from: the live system's read-only squashfs and the medium's
/// `/live/` directory (the image, its hash tree, kernel and initramfs). The live medium and the
/// recovery both boot through live-boot, which mounts them at the same places.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    /// The image's root, read-only (live-boot's mount of the squashfs, every block of it verified
    /// against its hash tree when the medium has one).
    pub root: String,
    /// The medium's `/live/` directory.
    pub live: String,
}

impl Image {
    /// What live-boot mounts, on the live medium and in the recovery alike.
    pub fn live_boot() -> Self {
        Self {
            root: "/run/live/rootfs/filesystem.squashfs".into(),
            live: "/run/live/medium/live".into(),
        }
    }

    pub fn squashfs(&self) -> String {
        format!("{}/filesystem.squashfs", self.live)
    }
}

/// systemd-repart's definitions of the disk, by file name, in the order repart reads them.
///
/// - The EFI system partition and OxyOS Recovery are exactly sized, so a run on a disk that has
///   them (an erase, `--factory-reset=yes`) matches them as they are and leaves them alone; only a
///   partition repart creates is formatted and filled.
/// - The recovery is filled with the medium's `/live/`: the image, its hash tree and root hash,
///   its kernel and live-boot initramfs.
/// - The root is filled with the image's tree. repart leaves out of it the contents of every other
///   partition's mount point (the EFI system partition's `/boot` and `/efi`), so `/boot`, where
///   Debian keeps the kernel, is copied by a line of its own. `encrypt`: LUKS2 with the key file
///   given to repart (ADR 0023, decision 3). `FactoryReset=yes`: an erase deletes it.
pub fn definitions(image: &Image, encrypt: bool) -> Vec<(String, String)> {
    let mut root = String::new();
    let _ = write!(
        root,
        "[Partition]\nType=root\nLabel={ROOT_LABEL}\nSizeMinBytes={ROOT_MIN_SIZE}\nFormat=ext4\n"
    );
    if encrypt {
        root.push_str("Encrypt=key-file\n");
    }
    let _ = write!(
        root,
        "CopyFiles={image_root}:/\nCopyFiles={image_root}/boot:/boot\nFactoryReset=yes\n",
        image_root = image.root
    );
    vec![
        (
            "00-esp.conf".into(),
            format!(
                "[Partition]\nType=esp\nLabel={ESP_LABEL}\nSizeMinBytes={ESP_SIZE}\nSizeMaxBytes={ESP_SIZE}\nFormat=vfat\n"
            ),
        ),
        (
            "10-recovery.conf".into(),
            format!(
                "[Partition]\nType={RECOVERY_TYPE}\nLabel={RECOVERY_LABEL}\nSizeMinBytes={RECOVERY_SIZE}\nSizeMaxBytes={RECOVERY_SIZE}\nFormat=ext4\nCopyFiles={live}:/live\n",
                live = image.live
            ),
        ),
        ("20-root.conf".into(), root),
    ]
}

/// Options of the root's ext4 for systemd-repart (`SYSTEMD_REPART_MKFS_OPTIONS_EXT4`): the
/// `encrypt` feature, so a home can later be encrypted with a key of its own without reformatting
/// (ADR 0023, decision 2).
pub const EXT4_OPTIONS: &str = "-O encrypt";

/// The kernel command line of the installed system (`/etc/kernel/cmdline`, which kernel-install
/// writes into each entry): the root, and the boot logo's (ADR 0017).
pub fn kernel_command_line(root_device: &str) -> String {
    format!(
        "root={root_device} rw quiet splash loglevel=3 vt.global_cursor_default=0 plymouth.ignore-serial-consoles"
    )
}

/// systemd-boot's configuration (`loader/loader.conf`):
/// - the menu hidden, shown when a key is held as the machine starts;
/// - the first entry that may boot is the default (`oxyos-*`, the newest kernel first and the
///   recovery after them: an entry with no tries left goes to the end, so the recovery comes
///   first once every kernel has failed);
/// - only OxyOS's entries: none for the firmware's own loader, its setup or other systems found
///   on the disk;
/// - no editing of the command line at the menu: what a person at the keyboard can change of the
///   boot is which entry starts, not what the kernel is told.
pub const LOADER_CONF: &str =
    "timeout menu-hidden\ndefault oxyos-*\neditor no\nauto-entries no\nauto-firmware no\n";

/// The recovery's entry on the EFI system partition.
pub const RECOVERY_ENTRY: &str = "oxyos-recovery.conf";
/// The recovery's kernel and initramfs on the EFI system partition, from its root.
pub const RECOVERY_DIRECTORY: &str = "oxyos-recovery";

/// The recovery's boot entry (Boot Loader Specification Type 1): the medium's kernel and live-boot
/// initramfs, booting the image on OxyOS Recovery (found by its partition UUID) read-only, every
/// block verified against `root_hash`, into `oxyos-recovery.target`. `live-config.nocomponents`:
/// live-config makes no live user and logs nobody in, the recovery is its own program on the
/// console; `noeject`: restarting asks for no medium to be removed (the medium is the disk).
pub fn recovery_entry(recovery_partuuid: &str, root_hash: &str, version: &str) -> String {
    format!(
        "title      OxyOS Recovery\nsort-key   oxyos-recovery\nversion    {version}\nlinux      /{RECOVERY_DIRECTORY}/linux\ninitrd     /{RECOVERY_DIRECTORY}/initrd.img\noptions    boot=live live-media=/dev/disk/by-partuuid/{recovery_partuuid} live-media-path=/live noeject live-config.nocomponents dm-verity-root-hash=filesystem.squashfs:{root_hash} systemd.unit=oxyos-recovery.target systemd.hostname=oxyos-recovery quiet splash loglevel=3 vt.global_cursor_default=0 plymouth.ignore-serial-consoles\n"
    )
}

/// The installed system's `/etc/fstab`: the root and the EFI system partition at `/efi`, readable
/// by root alone (it holds the install key's initramfs until setup, ADR 0023 decision 3).
pub fn fstab(root_fs_uuid: &str, esp_fs_uuid: &str) -> String {
    format!(
        "UUID={root_fs_uuid} / ext4 errors=remount-ro 0 1\nUUID={esp_fs_uuid} /efi vfat umask=0077,shortname=winnt 0 2\n"
    )
}

/// The installed system's `/etc/crypttab`: the root's LUKS2 volume, unlocked by the install key
/// until setup gives it the accounts' passwords (ADR 0023, decision 3), or by a passphrase when
/// `key_file` is none.
pub fn crypttab(luks_uuid: &str, key_file: Option<&Path>) -> String {
    let key = key_file.map_or_else(|| "none".to_string(), |p| p.display().to_string());
    format!("{ROOT_MAPPER} UUID={luks_uuid} {key} luks,discard\n")
}

/// Where the install key is kept on the installed system; cryptsetup-initramfs copies it into the
/// initramfs (`KEYFILE_PATTERN`), so the first boot unlocks with no password.
pub const INSTALL_KEY: &str = "/etc/cryptsetup-keys.d/oxyos-root.key";
