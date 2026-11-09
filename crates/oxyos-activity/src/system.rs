//! The machine's use as a whole: the processors (`/proc/stat`), the load (`/proc/loadavg`),
//! memory (`/proc/meminfo`, its pressure from `/proc/pressure/memory`, what zram compressed), the
//! disks (`/proc/diskstats`, the block devices that are hardware) and the network's interfaces
//! (`/proc/net/dev`). Counters since boot; the sampler makes rates of two samples.

use crate::procfs::{read_into, Procfs};
use serde::Serialize;

/// The processors' time since boot, in ticks, as Activity Monitor splits it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Processors {
    /// `user` and `nice` (a guest's time is counted in them).
    pub user: u64,
    /// `system`, `irq` and `softirq`.
    pub system: u64,
    /// `idle` and `iowait`.
    pub idle: u64,
    pub cores: u32,
}

pub fn processors(text: &str) -> Option<Processors> {
    let mut found = None;
    let mut cores = 0;
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        match fields.next() {
            Some("cpu") => {
                let n: Vec<u64> = fields.filter_map(|f| f.parse().ok()).collect();
                let at = |i: usize| n.get(i).copied().unwrap_or(0);
                found = Some(Processors {
                    user: at(0) + at(1),
                    system: at(2) + at(5) + at(6),
                    idle: at(3) + at(4),
                    cores: 0,
                });
            }
            Some(name) if name.starts_with("cpu") => cores += 1,
            _ => {}
        }
    }
    found.map(|p| Processors { cores, ..p })
}

/// The time the system booted, in seconds since the epoch (`/proc/stat`'s `btime`).
pub fn boot_time(text: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.strip_prefix("btime ")?.trim().parse().ok())
}

/// `/proc/meminfo`, in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Meminfo {
    pub total: u64,
    pub available: u64,
    pub anonymous: u64,
    pub shmem: u64,
    pub cached: u64,
    pub buffers: u64,
    /// The kernel's own that is never reclaimed: unreclaimable slab, stacks and page tables.
    pub wired: u64,
    pub swap_total: u64,
    pub swap_free: u64,
    /// What zswap's compressed pool holds.
    pub zswap: u64,
}

pub fn meminfo(text: &str) -> Meminfo {
    let mut m = Meminfo::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let bytes = value
            .split_whitespace()
            .next()
            .and_then(|n| n.parse::<u64>().ok())
            .unwrap_or(0)
            * 1024;
        match key {
            "MemTotal" => m.total = bytes,
            "MemAvailable" => m.available = bytes,
            "AnonPages" => m.anonymous = bytes,
            "Shmem" => m.shmem = bytes,
            "Cached" => m.cached = bytes,
            "Buffers" => m.buffers = bytes,
            "SUnreclaim" | "KernelStack" | "PageTables" | "SecPageTables" => m.wired += bytes,
            "SwapTotal" => m.swap_total = bytes,
            "SwapFree" => m.swap_free = bytes,
            "Zswap" => m.zswap = bytes,
            _ => {}
        }
    }
    m
}

/// Microseconds some and all of the tasks stalled waiting for memory since boot (the kernel's
/// pressure stall information, `/proc/pressure/memory`'s `total`s).
pub fn pressure(text: &str) -> Option<(u64, u64)> {
    let total = |kind: &str| {
        text.lines()
            .find(|line| line.starts_with(kind))?
            .split_whitespace()
            .find_map(|field| field.strip_prefix("total=")?.parse::<u64>().ok())
    };
    Some((total("some")?, total("full")?))
}

/// What a disk or the network moved since boot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Moved {
    /// Bytes read, or received.
    pub bytes_in: u64,
    /// Bytes written, or sent.
    pub bytes_out: u64,
    /// Reads, or packets received.
    pub count_in: u64,
    /// Writes, or packets sent.
    pub count_out: u64,
}

impl Moved {
    fn plus(self, other: Moved) -> Moved {
        Moved {
            bytes_in: self.bytes_in + other.bytes_in,
            bytes_out: self.bytes_out + other.bytes_out,
            count_in: self.count_in + other.count_in,
            count_out: self.count_out + other.count_out,
        }
    }
}

/// The block devices that are hardware: those with a `device` (a disk, never a partition, a loop,
/// a RAM disk, zram or device-mapper's view of another, which would count its I/O twice).
pub fn disks(procfs: &Procfs) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(procfs.sys.join("block")) else {
        return Vec::new();
    };
    let mut disks: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.path().join("device").exists())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| !name.starts_with("zram"))
        .collect();
    disks.sort();
    disks
}

/// What `disks` read and wrote since boot (`/proc/diskstats`: sectors of 512 bytes).
pub fn diskstats(text: &str, disks: &[String]) -> Moved {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if !disks
                .iter()
                .any(|disk| Some(&disk.as_str()) == fields.get(2))
            {
                return None;
            }
            let at = |i: usize| fields.get(i)?.parse::<u64>().ok();
            Some(Moved {
                count_in: at(3)?,
                bytes_in: at(5)? * 512,
                count_out: at(7)?,
                bytes_out: at(9)? * 512,
            })
        })
        .fold(Moved::default(), Moved::plus)
}

/// Each network interface but the loopback, and what it moved since it came up.
pub fn interfaces(text: &str) -> Vec<(String, Moved)> {
    text.lines()
        .skip(2)
        .filter_map(|line| {
            let (name, counters) = line.split_once(':')?;
            let name = name.trim();
            if name == "lo" {
                return None;
            }
            let n: Vec<u64> = counters
                .split_whitespace()
                .filter_map(|f| f.parse().ok())
                .collect();
            let at = |i: usize| n.get(i).copied();
            Some((
                name.to_owned(),
                Moved {
                    bytes_in: at(0)?,
                    count_in: at(1)?,
                    bytes_out: at(8)?,
                    count_out: at(9)?,
                },
            ))
        })
        .collect()
}

/// What zram's devices hold, compressed, in bytes (`mm_stat`'s third field, the memory it uses).
pub fn zram(procfs: &Procfs, buffer: &mut String) -> u64 {
    let Ok(entries) = std::fs::read_dir(procfs.sys.join("block")) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("zram"))
        .filter_map(|entry| {
            read_into(&entry.path().join("mm_stat"), buffer)?;
            buffer.split_whitespace().nth(2)?.parse::<u64>().ok()
        })
        .sum()
}

/// Memory pressure as a Mac's graph colours it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Normal,
    Warning,
    Critical,
}

/// The level of a share (percent) of the interval in which some tasks, and all of them, stalled
/// for memory: from a tenth of the time some did, the machine is felt to slow (yellow); from 40 %
/// some, or 10 % all, it pages hard (red), short of the 60 % systemd-oomd ends a unit at.
pub fn level(some: f64, full: f64) -> Level {
    if some >= 40.0 || full >= 10.0 {
        Level::Critical
    } else if some >= 10.0 {
        Level::Warning
    } else {
        Level::Normal
    }
}
