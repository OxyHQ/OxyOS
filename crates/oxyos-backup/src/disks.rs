//! Backup disks ([ADR 0059](../../docs/adr/0059-backups.mdx), decision 3): which connected disks
//! can hold backups, knowing when one is connected or taken away, and mounting one while it is
//! used.
//!
//! - **Which disks.** A file system udev identified (`ID_FS_USAGE=filesystem`, with a UUID) of a
//!   kind the kernel writes safely and restic's files fit (ext4, btrfs, XFS, exFAT), on a disk
//!   that holds none of the system's mounted file systems (the root's disk, its EFI system
//!   partition and the recovery's are never offered), and not a loop, device-mapper, optical or
//!   memory device. A destination is its file system's UUID, as Time Machine knows its disk.
//! - **When.** udev's monitor (libudev, as the input devices are followed) tells the service as a
//!   disk comes or goes: its descriptor is waited on with the service's others, never polled.
//! - **Mounting.** The service mounts a disk in its own mount namespace (systemd's
//!   `PrivateMounts=`), at `/run/oxyos/backupd/volumes/<uuid>`, `nosuid,nodev,noexec`, only while
//!   it reads or writes it, and unmounts it after: the disk can be taken away whenever no backup
//!   runs, and nothing else on the machine sees it mounted.
//!
//! [`Played`] is the development service's and the tests': each folder of a directory is a disk
//! connected, its name the UUID, followed by inotify.

use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The file systems a backup disk may have.
pub const FILE_SYSTEMS: [&str; 4] = ["ext4", "btrfs", "xfs", "exfat"];

/// A disk that can hold backups.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Disk {
    /// Its file system's UUID.
    pub uuid: String,
    /// Its label, else what the person is shown of its size.
    pub label: String,
    pub fs_type: String,
    /// Its size in bytes.
    pub size: u64,
    #[serde(skip)]
    pub device: PathBuf,
}

/// Disks, as the service sees them.
pub trait Disks: Send + Sync {
    /// The disks connected now.
    fn list(&self) -> Vec<Disk>;
    /// Mounts the disk (if it is not), and says where.
    fn mount(&self, disk: &Disk) -> io::Result<PathBuf>;
    /// Unmounts the disk, once nothing uses it.
    fn unmount(&self, uuid: &str);
    /// Follows disks coming and going, on the thread that calls it.
    fn watch(&self) -> io::Result<Box<dyn Watch>>;
}

/// Disks coming and going.
pub trait Watch {
    /// A descriptor readable when disks may have come or gone.
    fn descriptor(&self) -> BorrowedFd<'_>;
    /// Reads what the descriptor said, so it waits again.
    fn changed(&self);
}

/// Bytes free for an unprivileged writer on the file system at `path`.
pub fn free_bytes(path: &Path) -> Option<u64> {
    let stat = rustix::fs::statvfs(path).ok()?;
    Some(stat.f_bavail.saturating_mul(stat.f_frsize))
}

/// The machine's disks, through udev.
pub struct System {
    volumes: PathBuf,
    mounted: Mutex<BTreeSet<String>>,
}

impl System {
    /// The machine's disks; they are mounted under `volumes`.
    pub fn open(volumes: &Path) -> io::Result<Self> {
        fs::create_dir_all(volumes)?;
        Ok(Self {
            volumes: volumes.to_owned(),
            mounted: Mutex::new(BTreeSet::new()),
        })
    }

    fn mount_point(&self, uuid: &str) -> io::Result<PathBuf> {
        if !valid_uuid(uuid) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a file system's UUID",
            ));
        }
        Ok(self.volumes.join(uuid))
    }
}

/// A file system's UUID as udev writes it: hexadecimal digits and dashes.
pub fn valid_uuid(uuid: &str) -> bool {
    !uuid.is_empty() && uuid.len() <= 64 && uuid.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

impl Disks for System {
    fn list(&self) -> Vec<Disk> {
        let system = system_disks(Path::new("/proc/self/mountinfo"), Path::new("/sys"));
        let Ok(mut enumerator) = udev::Enumerator::new() else {
            return Vec::new();
        };
        if enumerator.match_subsystem("block").is_err() {
            return Vec::new();
        }
        let Ok(devices) = enumerator.scan_devices() else {
            return Vec::new();
        };
        let mut disks = Vec::new();
        for device in devices {
            let property = |name: &str| {
                device
                    .property_value(name)
                    .map(|v| v.to_string_lossy().into_owned())
            };
            let name = device.sysname().to_string_lossy().into_owned();
            if ["loop", "dm-", "sr", "zram", "ram", "md"]
                .iter()
                .any(|prefix| name.starts_with(prefix))
            {
                continue;
            }
            if property("ID_FS_USAGE").as_deref() != Some("filesystem") {
                continue;
            }
            let (Some(uuid), Some(fs_type), Some(device_node)) = (
                property("ID_FS_UUID"),
                property("ID_FS_TYPE"),
                device.devnode(),
            ) else {
                continue;
            };
            if !FILE_SYSTEMS.contains(&fs_type.as_str()) || !valid_uuid(&uuid) {
                continue;
            }
            let disk = match device.devtype().and_then(|t| t.to_str()) {
                Some("partition") => device
                    .parent()
                    .map(|p| p.sysname().to_string_lossy().into_owned()),
                _ => Some(name.clone()),
            };
            if disk.is_none_or(|disk| system.contains(&disk)) {
                continue;
            }
            let size = device
                .attribute_value("size")
                .and_then(|s| s.to_str()?.trim().parse::<u64>().ok())
                .unwrap_or(0)
                .saturating_mul(512);
            disks.push(Disk {
                uuid,
                label: property("ID_FS_LABEL").unwrap_or_default(),
                fs_type,
                size,
                device: device_node.to_owned(),
            });
        }
        disks.sort_by(|a, b| a.label.cmp(&b.label).then(a.uuid.cmp(&b.uuid)));
        disks
    }

    fn mount(&self, disk: &Disk) -> io::Result<PathBuf> {
        let target = self.mount_point(&disk.uuid)?;
        let mut mounted = self.mounted.lock().unwrap_or_else(|e| e.into_inner());
        if mounted.contains(&disk.uuid) {
            return Ok(target);
        }
        fs::create_dir_all(&target)?;
        use rustix::mount::MountFlags;
        rustix::mount::mount(
            &disk.device,
            &target,
            disk.fs_type.as_str(),
            MountFlags::NOSUID | MountFlags::NODEV | MountFlags::NOEXEC,
            None,
        )?;
        mounted.insert(disk.uuid.clone());
        Ok(target)
    }

    fn unmount(&self, uuid: &str) {
        let mut mounted = self.mounted.lock().unwrap_or_else(|e| e.into_inner());
        if mounted.remove(uuid) {
            if let Ok(target) = self.mount_point(uuid) {
                rustix::fs::sync();
                // A disk taken away mid-way cannot be unmounted cleanly: detached instead.
                if rustix::mount::unmount(&target, rustix::mount::UnmountFlags::empty()).is_err() {
                    let _ = rustix::mount::unmount(&target, rustix::mount::UnmountFlags::DETACH);
                }
                let _ = fs::remove_dir(&target);
            }
        }
    }

    fn watch(&self) -> io::Result<Box<dyn Watch>> {
        let monitor = udev::MonitorBuilder::new()?
            .match_subsystem("block")?
            .listen()?;
        Ok(Box::new(UdevWatch(monitor)))
    }
}

/// udev's monitor of block devices (libudev's, used by the thread that made it).
struct UdevWatch(udev::MonitorSocket);

impl Watch for UdevWatch {
    fn descriptor(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }

    fn changed(&self) {
        for _event in self.0.iter() {}
    }
}

/// The disks (by their kernel names: `nvme0n1`, `sda`) that hold a file system mounted now, the
/// device-mapper's (an encrypted root) followed to the disks below them.
pub fn system_disks(mountinfo: &Path, sys: &Path) -> BTreeSet<String> {
    let mut disks = BTreeSet::new();
    let Ok(text) = fs::read_to_string(mountinfo) else {
        return disks;
    };
    for line in text.lines() {
        // mountinfo(5): the third field is the device's major:minor.
        let Some(device) = line.split_whitespace().nth(2) else {
            continue;
        };
        let Some((major, _)) = device.split_once(':') else {
            continue;
        };
        if major == "0" {
            continue; // a file system with no device (proc, tmpfs, …)
        }
        let Ok(path) = fs::canonicalize(sys.join("dev/block").join(device)) else {
            continue;
        };
        below(&path, &mut disks, 0);
    }
    disks
}

/// The disks below a block device's sysfs folder.
fn below(device: &Path, disks: &mut BTreeSet<String>, depth: usize) {
    if depth > 8 {
        return;
    }
    let slaves = device.join("slaves");
    if let Ok(entries) = fs::read_dir(&slaves) {
        let mut any = false;
        for entry in entries.flatten() {
            any = true;
            if let Ok(path) = fs::canonicalize(entry.path()) {
                below(&path, disks, depth + 1);
            }
        }
        if any {
            return;
        }
    }
    let disk = if device.join("partition").exists() {
        device.parent()
    } else {
        Some(device)
    };
    if let Some(name) = disk.and_then(Path::file_name) {
        disks.insert(name.to_string_lossy().into_owned());
    }
}

/// Disks played by folders: each folder of a directory is a connected disk, its name the UUID, its
/// label the contents of its `.oxyos-label` if it has one. Followed by inotify.
pub struct Played {
    directory: PathBuf,
}

impl Played {
    pub fn open(directory: &Path) -> io::Result<Self> {
        fs::create_dir_all(directory)?;
        Ok(Self {
            directory: directory.to_owned(),
        })
    }
}

/// inotify on the played disks' directory.
struct PlayedWatch(OwnedFd);

impl Watch for PlayedWatch {
    fn descriptor(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }

    fn changed(&self) {
        let mut buffer = [std::mem::MaybeUninit::<u8>::uninit(); 4096];
        while rustix::io::read(&self.0, &mut buffer).is_ok_and(|(read, _)| !read.is_empty()) {}
    }
}

impl Disks for Played {
    fn list(&self) -> Vec<Disk> {
        let Ok(entries) = fs::read_dir(&self.directory) else {
            return Vec::new();
        };
        let mut disks: Vec<Disk> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| {
                let uuid = e.file_name().to_str()?.to_owned();
                if !valid_uuid(&uuid) {
                    return None;
                }
                let label = fs::read_to_string(e.path().join(".oxyos-label"))
                    .map(|l| l.trim().to_owned())
                    .unwrap_or_default();
                Some(Disk {
                    uuid,
                    label,
                    fs_type: "ext4".into(),
                    size: 0,
                    device: e.path(),
                })
            })
            .collect();
        disks.sort_by(|a, b| a.label.cmp(&b.label).then(a.uuid.cmp(&b.uuid)));
        disks
    }

    fn mount(&self, disk: &Disk) -> io::Result<PathBuf> {
        let path = self.directory.join(&disk.uuid);
        if path.is_dir() {
            Ok(path)
        } else {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                "the disk is not connected",
            ))
        }
    }

    fn unmount(&self, _uuid: &str) {}

    fn watch(&self) -> io::Result<Box<dyn Watch>> {
        use rustix::fs::inotify;
        let fd = inotify::init(inotify::CreateFlags::CLOEXEC | inotify::CreateFlags::NONBLOCK)?;
        inotify::add_watch(
            &fd,
            &self.directory,
            inotify::WatchFlags::CREATE
                | inotify::WatchFlags::DELETE
                | inotify::WatchFlags::MOVED_TO
                | inotify::WatchFlags::MOVED_FROM,
        )?;
        Ok(Box::new(PlayedWatch(fd)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_disks_are_those_below_every_mounted_file_system() {
        let dir = tempfile::tempdir().unwrap();
        let sys = dir.path().join("sys");
        // nvme0n1p2 under LUKS (dm-0) is the root; nvme0n1p1 the EFI system partition; sda1 a USB
        // disk nobody mounted.
        let devices = sys.join("devices");
        let nvme = devices.join("pci/nvme/nvme0n1");
        fs::create_dir_all(nvme.join("nvme0n1p1")).unwrap();
        fs::create_dir_all(nvme.join("nvme0n1p2")).unwrap();
        fs::write(nvme.join("nvme0n1p1/partition"), "1").unwrap();
        fs::write(nvme.join("nvme0n1p2/partition"), "2").unwrap();
        let dm = devices.join("virtual/block/dm-0");
        fs::create_dir_all(dm.join("slaves")).unwrap();
        std::os::unix::fs::symlink(nvme.join("nvme0n1p2"), dm.join("slaves/nvme0n1p2")).unwrap();
        let sda = devices.join("usb/sda");
        fs::create_dir_all(sda.join("sda1")).unwrap();
        fs::write(sda.join("sda1/partition"), "1").unwrap();
        let block = sys.join("dev/block");
        fs::create_dir_all(&block).unwrap();
        std::os::unix::fs::symlink(&dm, block.join("254:0")).unwrap();
        std::os::unix::fs::symlink(nvme.join("nvme0n1p1"), block.join("259:1")).unwrap();
        std::os::unix::fs::symlink(sda.join("sda1"), block.join("8:1")).unwrap();
        let mountinfo = dir.path().join("mountinfo");
        fs::write(
            &mountinfo,
            "22 1 254:0 / / rw - ext4 /dev/mapper/root rw\n\
             23 22 259:1 / /efi rw - vfat /dev/nvme0n1p1 rw\n\
             24 22 0:22 / /proc rw - proc proc rw\n",
        )
        .unwrap();
        let disks = system_disks(&mountinfo, &sys);
        assert_eq!(disks.into_iter().collect::<Vec<_>>(), vec!["nvme0n1"]);
    }

    #[test]
    fn played_disks_come_and_go_and_are_said_to() {
        let dir = tempfile::tempdir().unwrap();
        let played = Played::open(dir.path()).unwrap();
        let watch = played.watch().unwrap();
        assert!(played.list().is_empty());
        fs::create_dir(dir.path().join("1234-ABCD")).unwrap();
        fs::write(dir.path().join("1234-ABCD/.oxyos-label"), "Backup\n").unwrap();
        fs::create_dir(dir.path().join("not a uuid")).unwrap();
        let mut fds = [rustix::event::PollFd::from_borrowed_fd(
            watch.descriptor(),
            rustix::event::PollFlags::IN,
        )];
        assert_eq!(rustix::event::poll(&mut fds, None).unwrap(), 1);
        watch.changed();
        let disks = played.list();
        assert_eq!(disks.len(), 1);
        assert_eq!(disks[0].label, "Backup");
        assert_eq!(
            played.mount(&disks[0]).unwrap(),
            dir.path().join("1234-ABCD")
        );
        assert!(valid_uuid("2f1e-ab"));
        assert!(!valid_uuid("../x"));
    }
}
