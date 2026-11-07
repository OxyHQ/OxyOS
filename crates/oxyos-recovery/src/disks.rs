//! The machine's disks, as `lsblk --json` sees them: the ones an install may take, and the OxyOS
//! layout on one (its EFI system partition, recovery and root, each by its type).

use crate::layout::{self, ESP_TYPE, RECOVERY_TYPE};
use serde::Deserialize;

/// What `lsblk` is asked for: [`parse`] reads exactly this.
pub const LSBLK_ARGUMENTS: &[&str] = &[
    "--json",
    "--bytes",
    "--tree",
    "--output",
    "NAME,PATH,TYPE,SIZE,RM,RO,TRAN,MODEL,PTUUID,PARTTYPE,PARTUUID,PARTLABEL,FSTYPE,UUID,MOUNTPOINTS",
];

/// A whole disk and its partitions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disk {
    pub path: String,
    pub size: u64,
    pub removable: bool,
    pub read_only: bool,
    /// How it is attached (`nvme`, `sata`, `usb`), when the kernel says.
    pub transport: Option<String>,
    pub model: Option<String>,
    /// The partition table's UUID (GPT's disk GUID).
    pub ptuuid: Option<String>,
    pub partitions: Vec<Partition>,
}

/// A partition, and what is opened on it (a LUKS2 volume's mapping).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Partition {
    pub path: String,
    pub size: u64,
    pub parttype: Option<String>,
    pub partuuid: Option<String>,
    pub partlabel: Option<String>,
    /// The file system or container on it (`vfat`, `ext4`, `crypto_LUKS`).
    pub fstype: Option<String>,
    /// That file system's or container's UUID.
    pub uuid: Option<String>,
    pub mountpoints: Vec<String>,
    /// What is opened on it: a LUKS2 volume's mapping, with its file system and mount points.
    pub holders: Vec<Partition>,
}

impl Partition {
    fn of_type(&self, wanted: &str) -> bool {
        self.parttype
            .as_deref()
            .is_some_and(|t| t.eq_ignore_ascii_case(wanted))
    }

    pub fn encrypted(&self) -> bool {
        self.fstype.as_deref() == Some("crypto_LUKS")
    }

    /// Whether this partition, or what is opened on it, is mounted at `path`.
    pub fn mounted_at(&self, path: &str) -> bool {
        self.mountpoints.iter().any(|m| m == path)
            || self.holders.iter().any(|h| h.mounted_at(path))
    }
}

/// The OxyOS layout found on a disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub disk: Disk,
    pub esp: Partition,
    pub recovery: Partition,
    pub root: Partition,
}

impl Disk {
    /// The one partition of `parttype` on it; none when there is none or more than one, which no
    /// install writes and nothing here guesses between.
    pub fn only(&self, parttype: &str) -> Option<&Partition> {
        let mut found = self.partitions.iter().filter(|p| p.of_type(parttype));
        let first = found.next()?;
        found.next().is_none().then_some(first)
    }

    /// OxyOS's layout on this disk, when it has one: one EFI system partition, one recovery and
    /// one root of this machine's architecture.
    pub fn layout(&self) -> Option<Layout> {
        self.layout_for(layout::root_type())
    }

    pub fn layout_for(&self, root_type: &str) -> Option<Layout> {
        Some(Layout {
            esp: self.only(ESP_TYPE)?.clone(),
            recovery: self.only(RECOVERY_TYPE)?.clone(),
            root: self.only(root_type)?.clone(),
            disk: self.clone(),
        })
    }

    /// Whether anything on this disk is mounted at `path`.
    pub fn mounted_at(&self, path: &str) -> bool {
        self.partitions.iter().any(|p| p.mounted_at(path))
    }

    /// A line naming it for a person: its path, size and model.
    pub fn describe(&self) -> String {
        let size = self.size as f64 / 1e9;
        let model = self
            .model
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .unwrap_or("disk");
        let attached = match self.transport.as_deref() {
            Some("usb") => ", USB",
            _ if self.removable => ", removable",
            _ => "",
        };
        format!("{} — {model}, {size:.1} GB{attached}", self.path)
    }
}

/// Why a disk cannot take an install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unsuitable {
    /// It holds what this system booted from (the live medium, or the recovery itself).
    BootMedium,
    ReadOnly,
    TooSmall {
        size: u64,
    },
}

/// Whether `disk` may take an install, for a system booted from what is mounted at `medium` (the
/// live medium's or the recovery's mount point, `/run/live/medium`).
pub fn suitable(disk: &Disk, medium: &str) -> Result<(), Unsuitable> {
    if disk.mounted_at(medium) {
        return Err(Unsuitable::BootMedium);
    }
    if disk.read_only {
        return Err(Unsuitable::ReadOnly);
    }
    if disk.size < layout::MIN_DISK_SIZE {
        return Err(Unsuitable::TooSmall { size: disk.size });
    }
    Ok(())
}

/// The disk holding what is mounted at `path` (the medium this system booted from, or `/`).
pub fn disk_with_mount<'a>(disks: &'a [Disk], path: &str) -> Option<&'a Disk> {
    disks.iter().find(|d| d.mounted_at(path))
}

#[derive(Deserialize)]
struct Answer {
    blockdevices: Vec<Device>,
}

#[derive(Deserialize)]
struct Device {
    path: String,
    #[serde(rename = "type")]
    kind: String,
    size: Option<u64>,
    #[serde(default, deserialize_with = "flag")]
    rm: bool,
    #[serde(default, deserialize_with = "flag")]
    ro: bool,
    tran: Option<String>,
    model: Option<String>,
    ptuuid: Option<String>,
    parttype: Option<String>,
    partuuid: Option<String>,
    partlabel: Option<String>,
    fstype: Option<String>,
    uuid: Option<String>,
    #[serde(default)]
    mountpoints: Vec<Option<String>>,
    #[serde(default)]
    children: Vec<Device>,
}

/// lsblk writes its flags as JSON booleans (util-linux 2.37 and later) or as "0"/"1".
fn flag<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Flag {
        Bool(bool),
        Text(String),
    }
    Ok(match Option::<Flag>::deserialize(deserializer)? {
        Some(Flag::Bool(b)) => b,
        Some(Flag::Text(t)) => t == "1",
        None => false,
    })
}

impl Device {
    fn partition(self) -> Partition {
        Partition {
            path: self.path,
            size: self.size.unwrap_or(0),
            parttype: self.parttype,
            partuuid: self.partuuid,
            partlabel: self.partlabel,
            fstype: self.fstype,
            uuid: self.uuid,
            mountpoints: self.mountpoints.into_iter().flatten().collect(),
            holders: self.children.into_iter().map(Device::partition).collect(),
        }
    }
}

/// The disks in `lsblk`'s answer to [`LSBLK_ARGUMENTS`]: whole disks only (no loop devices,
/// optical drives, RAM disks or zram), each with its partitions.
pub fn parse(json: &str) -> Result<Vec<Disk>, String> {
    let answer: Answer = serde_json::from_str(json).map_err(|e| format!("lsblk's answer: {e}"))?;
    Ok(answer
        .blockdevices
        .into_iter()
        .filter(|d| d.kind == "disk" && !d.path.starts_with("/dev/zram"))
        .map(|d| Disk {
            path: d.path,
            size: d.size.unwrap_or(0),
            removable: d.rm,
            read_only: d.ro,
            transport: d.tran,
            model: d.model,
            ptuuid: d.ptuuid,
            partitions: d
                .children
                .into_iter()
                .filter(|c| c.kind == "part")
                .map(Device::partition)
                .collect(),
        })
        .collect())
}
