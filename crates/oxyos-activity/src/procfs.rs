//! What `/proc` says of one process, each file read whole in one call and parsed here: the reads
//! Activity Monitor makes of every process every two seconds are its whole cost, so each is the
//! cheapest file that says what a column needs (docs/adr/0063-activity-monitor.mdx, decision 3).

use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// Where the kernel's files are: the system's, or a test's own trees.
#[derive(Clone, Debug)]
pub struct Procfs {
    /// `/proc`.
    pub proc: PathBuf,
    /// `/sys`, for which block devices are disks.
    pub sys: PathBuf,
    /// Where the users' names come from: the running system's account database through NSS
    /// (`None`), or a test's own passwd(5) file.
    pub passwd: Option<PathBuf>,
    /// The clock's ticks a second (`sysconf(_SC_CLK_TCK)`), which `stat`'s times count.
    pub ticks: u64,
}

impl Procfs {
    pub fn system() -> Self {
        Self {
            proc: PathBuf::from("/proc"),
            sys: PathBuf::from("/sys"),
            passwd: None,
            ticks: rustix::param::clock_ticks_per_second(),
        }
    }

    /// The process `pid`'s file `name`.
    pub fn of(&self, pid: u32, name: &str) -> PathBuf {
        self.proc.join(pid.to_string()).join(name)
    }

    /// The `/proc` the battery's energy reads (`oxyos_power::energy`), on the same tree.
    pub fn energy(&self) -> oxyos_power::energy::Proc<'_> {
        oxyos_power::energy::Proc {
            root: &self.proc,
            ticks: self.ticks,
        }
    }

    /// The pids `/proc` lists now.
    pub fn pids(&self) -> Vec<u32> {
        let Ok(entries) = std::fs::read_dir(&self.proc) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| entry.file_name().to_str()?.parse().ok())
            .collect()
    }
}

/// A file read whole into `buffer` (reused between reads), or nothing once it is gone.
pub fn read_into(path: &Path, buffer: &mut String) -> Option<std::fs::File> {
    buffer.clear();
    let mut file = std::fs::File::open(path).ok()?;
    file.read_to_string(buffer).ok()?;
    Some(file)
}

/// What `/proc/PID/stat` says, and who the process is (the file's owner: its effective user).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stat {
    pub pid: u32,
    /// Its `comm`: at most 15 bytes, cut.
    pub comm: String,
    pub ppid: u32,
    /// Its own processor time, user and system, in ticks (not its children's).
    pub cpu_ticks: u64,
    pub threads: u32,
    /// When it started, in ticks since boot: with the pid, what names it exactly.
    pub started: u64,
    /// Its resident pages.
    pub rss_pages: u64,
    pub uid: u32,
}

/// Parses `stat`'s text. The name is in parentheses and may hold spaces and parentheses: the
/// fields are what follows the last.
pub fn parse_stat(pid: u32, text: &str, uid: u32) -> Option<Stat> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let comm = text.get(open + 1..close)?.to_owned();
    let fields: Vec<&str> = text.get(close + 1..)?.split_whitespace().collect();
    // stat's fields counted from 1; here the 3rd (its state) is the first.
    let field = |n: usize| fields.get(n - 3)?.parse::<u64>().ok();
    Some(Stat {
        pid,
        comm,
        ppid: field(4)? as u32,
        cpu_ticks: field(14)? + field(15)?,
        threads: field(20)? as u32,
        started: field(22)?,
        rss_pages: field(24)?,
        uid,
    })
}

/// Reads the process `pid`'s stat.
pub fn stat(procfs: &Procfs, pid: u32, buffer: &mut String) -> Option<Stat> {
    let file = read_into(&procfs.of(pid, "stat"), buffer)?;
    let uid = file.metadata().ok()?.uid();
    parse_stat(pid, buffer, uid)
}

/// What `/proc/PID/status` says of its users and memory, in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// Real, effective, saved and file system user.
    pub uids: [u32; 4],
    /// Its anonymous pages resident (`RssAnon`).
    pub anonymous: u64,
    /// Its pages swapped out (`VmSwap`).
    pub swap: u64,
    /// Its resident set (`VmRSS`).
    pub resident: u64,
}

pub fn parse_status(text: &str) -> Status {
    let mut status = Status::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let kib = || {
            value
                .split_whitespace()
                .next()
                .and_then(|n| n.parse::<u64>().ok())
                .map_or(0, |n| n * 1024)
        };
        match key {
            "Uid" => {
                for (slot, id) in status.uids.iter_mut().zip(value.split_whitespace()) {
                    *slot = id.parse().unwrap_or(u32::MAX);
                }
            }
            "RssAnon" => status.anonymous = kib(),
            "VmSwap" => status.swap = kib(),
            "VmRSS" => status.resident = kib(),
            _ => {}
        }
    }
    status
}

pub fn status(procfs: &Procfs, pid: u32, buffer: &mut String) -> Option<Status> {
    read_into(&procfs.of(pid, "status"), buffer)?;
    Some(parse_status(buffer))
}

/// What a process read from and wrote to storage (`/proc/PID/io`'s `read_bytes` and
/// `write_bytes`, less what it wrote and truncated before it reached the disk), in bytes. Only
/// the process's own user may read it.
pub fn io(procfs: &Procfs, pid: u32, buffer: &mut String) -> Option<(u64, u64)> {
    read_into(&procfs.of(pid, "io"), buffer)?;
    let (mut read, mut written, mut cancelled) = (None, None, 0);
    for line in buffer.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().parse::<u64>().ok();
        match key {
            "read_bytes" => read = value,
            "write_bytes" => written = value,
            "cancelled_write_bytes" => cancelled = value.unwrap_or(0),
            _ => {}
        }
    }
    Some((read?, written?.saturating_sub(cancelled)))
}

/// How many times the process's threads were run on a processor since they started: each one's
/// `schedstat` third field (`pcount`). A thread that sleeps and wakes is run once per waking, so
/// its rate is the process's wakeups (docs/adr/0063-activity-monitor.mdx, decision 3).
pub fn runs(procfs: &Procfs, pid: u32, buffer: &mut String) -> Option<u64> {
    let tasks = std::fs::read_dir(procfs.of(pid, "task")).ok()?;
    let mut runs = 0;
    let mut any = false;
    for task in tasks.flatten() {
        if read_into(&task.path().join("schedstat"), buffer).is_none() {
            continue;
        }
        if let Some(count) = buffer
            .split_whitespace()
            .nth(2)
            .and_then(|n| n.parse::<u64>().ok())
        {
            runs += count;
            any = true;
        }
    }
    any.then_some(runs)
}

/// The process's command's name when `comm` was cut at 15 bytes: its first argument's last part.
/// A kernel thread, which has none, keeps `comm`.
pub fn full_name(procfs: &Procfs, pid: u32, comm: &str) -> String {
    if comm.len() < 15 {
        return comm.to_owned();
    }
    let Ok(arguments) = std::fs::read(procfs.of(pid, "cmdline")) else {
        return comm.to_owned();
    };
    let first = arguments.split(|b| *b == 0).next().unwrap_or_default();
    let name = String::from_utf8_lossy(first);
    let name = name.rsplit('/').next().unwrap_or_default();
    // Only a name `comm` is the start of: a process that renamed itself keeps the name it chose.
    if name.starts_with(comm) {
        name.to_owned()
    } else {
        comm.to_owned()
    }
}
