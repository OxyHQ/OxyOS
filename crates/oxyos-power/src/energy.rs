//! The energy each application uses (docs/adr/0057-battery-and-energy.mdx, decision 7): the
//! processor's time and the GPU's of every process the session started for it (the process it
//! launched and those it started in turn), read from `/proc`, as `top` and `intel_gpu_top` read
//! them:
//!
//! - the processor: each process's `utime` and `stime`, and its children's that it waited for
//!   (`cutime`, `cstime`), in the clock's ticks (`/proc/PID/stat`);
//! - the GPU: each DRM client's engines' busy time (`drm-engine-NAME: N ns` in
//!   `/proc/PID/fdinfo/FD`, which i915, xe's successor and amdgpu say), each client counted once
//!   however many descriptors it has open (`drm-pdev` and `drm-client-id`).
//!
//! It is sampled when the battery changes (UPower's signal: the history's moment) and when the
//! person asks (the battery's menu opening, Settings' page), never on a clock. An application
//! *uses significant energy* when, since the newest sample at least a minute old (or since it
//! started), its processor and GPU time together were at least a fifth of one core's.

use serde::Serialize;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::Path;
use std::time::Duration;

/// A fifth of a core, on average, is significant.
pub const SIGNIFICANT: f64 = 0.2;
/// The shortest window a use is averaged over, but since an application started.
pub const WINDOW: Duration = Duration::from_secs(60);
/// The samples kept per application: a minute's at Activity Monitor's pace of one every two seconds
/// (docs/adr/0063-activity-monitor.mdx), and more.
const KEPT: usize = 64;
/// The span an application's average energy covers, as a Mac's *12 hr Power*.
pub const AVERAGE: Duration = Duration::from_secs(12 * 60 * 60);
/// How far apart the checkpoints the average is taken from are, at least.
const CHECKPOINT: Duration = Duration::from_secs(60 * 60);
/// The checkpoints kept per application: twelve hours' and the one before.
const CHECKPOINTS: usize = 14;
/// Processes followed below a launched one, at most.
const MAX_TREE: usize = 512;

/// Time used, in nanoseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub cpu_ns: u64,
    pub gpu_ns: u64,
}

impl Usage {
    fn total(self) -> u64 {
        self.cpu_ns + self.gpu_ns
    }

    fn plus(self, other: Usage) -> Usage {
        Usage {
            cpu_ns: self.cpu_ns + other.cpu_ns,
            gpu_ns: self.gpu_ns + other.gpu_ns,
        }
    }

    fn since(self, before: Usage) -> Usage {
        Usage {
            cpu_ns: self.cpu_ns.saturating_sub(before.cpu_ns),
            gpu_ns: self.gpu_ns.saturating_sub(before.gpu_ns),
        }
    }
}

/// A process's DRM clients: their engines' busy time and the memory they hold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Drm {
    pub busy_ns: u64,
    /// Bytes.
    pub memory: u64,
}

/// A drm-usage-stats amount (`1024 KiB`, `12 MiB`, `4096`: bytes without a unit) in bytes.
fn bytes(value: &str) -> u64 {
    let (amount, unit) = value.split_once(' ').unwrap_or((value, ""));
    let Ok(amount) = amount.trim().parse::<u64>() else {
        return 0;
    };
    let scale = match unit.trim() {
        "" => 1,
        "KiB" => 1 << 10,
        "MiB" => 1 << 20,
        "GiB" => 1 << 30,
        _ => return 0,
    };
    amount.saturating_mul(scale)
}

/// A process's `stat` fields after its name: its state is the first (`stat`'s third field).
fn stat_fields(text: &str) -> Option<Vec<&str>> {
    // The name is in parentheses and may hold spaces and parentheses: what follows the last.
    let after = &text[text.rfind(')')? + 1..];
    Some(after.split_whitespace().collect())
}

/// What `/proc` (or a test's tree at `root`) says of processes.
pub struct Proc<'a> {
    pub root: &'a Path,
    /// The clock's ticks a second (`sysconf(_SC_CLK_TCK)`).
    pub ticks: u64,
}

impl Proc<'_> {
    pub fn system() -> Proc<'static> {
        Proc {
            root: Path::new("/proc"),
            ticks: rustix::param::clock_ticks_per_second(),
        }
    }

    /// A process's own and its waited children's processor time, and when it started, in
    /// nanoseconds since boot.
    fn cpu(&self, pid: u32) -> Option<(u64, u64)> {
        let text = std::fs::read_to_string(self.root.join(pid.to_string()).join("stat")).ok()?;
        let fields = stat_fields(&text)?;
        // utime, stime, cutime, cstime are stat's 14th to 17th fields; starttime its 22nd; the
        // fields here begin at its 3rd.
        let field = |n: usize| fields.get(n - 3)?.parse::<u64>().ok();
        let ticks = field(14)? + field(15)? + field(16)? + field(17)?;
        let ns = |ticks: u64| ticks.saturating_mul(1_000_000_000) / self.ticks.max(1);
        Some((ns(ticks), ns(field(22)?)))
    }

    /// The processes a process started that still run, by its threads' `children`.
    fn children(&self, pid: u32) -> Vec<u32> {
        let tasks = self.root.join(pid.to_string()).join("task");
        let Ok(entries) = std::fs::read_dir(tasks) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|task| std::fs::read_to_string(task.path().join("children")).ok())
            .flat_map(|text| {
                text.split_whitespace()
                    .filter_map(|pid| pid.parse().ok())
                    .collect::<Vec<u32>>()
            })
            .collect()
    }

    /// A process's GPU busy time, each DRM client once (`seen`).
    fn gpu(&self, pid: u32, seen: &mut HashSet<(String, String)>) -> u64 {
        self.drm(pid, seen).busy_ns
    }

    /// What a process's DRM clients say of themselves (the kernel's drm-usage-stats, in
    /// `/proc/PID/fdinfo/FD` of each descriptor of a DRM device): their engines' busy time and the
    /// memory they hold, each client once (`seen`: `drm-pdev` and `drm-client-id`) however many
    /// descriptors name it. Activity Monitor's GPU columns read it too, so the GPU is counted one
    /// way (docs/adr/0063-activity-monitor.mdx).
    pub fn drm(&self, pid: u32, seen: &mut HashSet<(String, String)>) -> Drm {
        let directory = self.root.join(pid.to_string());
        let mut drm = Drm::default();
        let Ok(entries) = std::fs::read_dir(directory.join("fd")) else {
            return drm;
        };
        for entry in entries.flatten() {
            // Only a DRM device's descriptor has a DRM client's fdinfo.
            let Ok(target) = std::fs::read_link(entry.path()) else {
                continue;
            };
            if !target.starts_with("/dev/dri") {
                continue;
            }
            let Ok(text) =
                std::fs::read_to_string(directory.join("fdinfo").join(entry.file_name()))
            else {
                continue;
            };
            let mut client = (String::new(), String::new());
            let (mut engines, mut memory, mut legacy) = (0u64, 0u64, 0u64);
            for line in text.lines() {
                let Some((key, value)) = line.split_once(':') else {
                    continue;
                };
                let value = value.trim();
                match key {
                    "drm-pdev" => client.0 = value.to_owned(),
                    "drm-client-id" => client.1 = value.to_owned(),
                    _ if key.starts_with("drm-engine-")
                        && !key.starts_with("drm-engine-capacity") =>
                    {
                        if let Some(n) = value
                            .strip_suffix("ns")
                            .and_then(|n| n.trim().parse::<u64>().ok())
                        {
                            engines += n;
                        }
                    }
                    // Every region's memory (`drm-total-system0`, `drm-total-local0`); a driver
                    // from before them says `drm-memory-<region>`, counted only where it says no
                    // total, as perf.py counts it.
                    _ if key.starts_with("drm-total-") => memory += bytes(value),
                    _ if key.starts_with("drm-memory-") => legacy += bytes(value),
                    _ => {}
                }
            }
            if client.1.is_empty() || seen.insert(client) {
                drm.busy_ns += engines;
                drm.memory += if memory > 0 { memory } else { legacy };
            }
        }
        drm
    }

    /// The time the process `pid` and every process below it used, and how long ago it started
    /// (by `uptime`, in nanoseconds since boot); none once it is gone.
    pub fn usage(&self, pid: u32, uptime_ns: u64) -> Option<(Usage, Duration)> {
        let (_, started) = self.cpu(pid)?;
        let mut usage = Usage::default();
        let mut seen = HashSet::new();
        let mut queue = vec![pid];
        let mut visited = HashSet::new();
        while let Some(next) = queue.pop() {
            if !visited.insert(next) || visited.len() > MAX_TREE {
                continue;
            }
            if let Some((cpu, _)) = self.cpu(next) {
                usage.cpu_ns += cpu;
                usage.gpu_ns += self.gpu(next, &mut seen);
                queue.extend(self.children(next));
            }
        }
        Some((
            usage,
            Duration::from_nanos(uptime_ns.saturating_sub(started)),
        ))
    }

    /// Nanoseconds since boot (`/proc/uptime`).
    pub fn uptime_ns(&self) -> Option<u64> {
        let text = std::fs::read_to_string(self.root.join("uptime")).ok()?;
        let seconds: f64 = text.split_whitespace().next()?.parse().ok()?;
        Some((seconds * 1e9) as u64)
    }
}

/// An application the session started, running now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Running {
    pub pid: u32,
    pub id: String,
    pub name: String,
}

/// What a sample read of one process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Read {
    pub pid: u32,
    pub usage: Usage,
    /// How long ago it started.
    pub age: Duration,
}

#[derive(Debug, Default)]
struct App {
    name: String,
    /// Each of its processes' use as last read, the ended ones' too.
    processes: BTreeMap<u32, Usage>,
    /// The running ones and how long they had run when last read.
    running: BTreeMap<u32, Duration>,
    /// Its whole use at each sample (when, by the meter's clock).
    samples: VecDeque<(Duration, Usage)>,
    /// Its whole use at most once an hour (when, by the meter's clock), for its average.
    checkpoints: VecDeque<(Duration, Usage)>,
    /// When its first process started, by the meter's clock, as its first sample said.
    began: Option<Duration>,
}

impl App {
    fn total(&self) -> Usage {
        self.processes
            .values()
            .fold(Usage::default(), |sum, usage| sum.plus(*usage))
    }
}

/// An application's energy, as the battery's menu and Settings say it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Energy {
    pub id: String,
    pub name: String,
    /// Seconds of the processor and of the GPU it used this session.
    pub cpu: f64,
    pub gpu: f64,
    /// Its recent use, in shares of one core (processor and GPU together).
    pub recent: f64,
    pub significant: bool,
    pub running: bool,
    /// Its use over the last twelve hours ([`AVERAGE`]), or since it began when that was later, in
    /// shares of one core: a Mac's *12 hr Power*.
    pub average: f64,
}

/// The applications' use, sampled.
#[derive(Debug, Default)]
pub struct Meter {
    apps: BTreeMap<String, App>,
}

impl Meter {
    /// A sample at `at` (the session's clock) of what `reads` read of the applications `running`.
    pub fn sample(&mut self, at: Duration, running: &[Running], reads: &[Read]) {
        for app in self.apps.values_mut() {
            app.running.clear();
        }
        for process in running {
            let Some(read) = reads.iter().find(|read| read.pid == process.pid) else {
                continue;
            };
            let app = self.apps.entry(process.id.clone()).or_default();
            app.name.clone_from(&process.name);
            if app.began.is_none() {
                app.began = Some(at.saturating_sub(read.age));
            }
            app.processes.insert(read.pid, read.usage);
            app.running.insert(read.pid, read.age);
        }
        for app in self.apps.values_mut() {
            let total = app.total();
            if app.samples.back().map(|(_, usage)| *usage) != Some(total) || !app.running.is_empty()
            {
                app.samples.push_back((at, total));
            }
            while app.samples.len() > KEPT {
                app.samples.pop_front();
            }
            if app
                .checkpoints
                .back()
                .is_none_or(|(then, _)| at.saturating_sub(*then) >= CHECKPOINT)
            {
                app.checkpoints.push_back((at, total));
            }
            while app.checkpoints.len() > CHECKPOINTS {
                app.checkpoints.pop_front();
            }
        }
    }

    /// Every application this session ran, the most energy first, as of the sample at `at`.
    pub fn report(&self, at: Duration) -> Vec<Energy> {
        let mut report: Vec<Energy> = self
            .apps
            .iter()
            .map(|(id, app)| {
                let total = app.total();
                let running = !app.running.is_empty();
                let recent = if running { recent(app, at) } else { 0.0 };
                Energy {
                    id: id.clone(),
                    name: app.name.clone(),
                    cpu: total.cpu_ns as f64 / 1e9,
                    gpu: total.gpu_ns as f64 / 1e9,
                    recent,
                    significant: running && recent >= SIGNIFICANT,
                    running,
                    average: average(app, at),
                }
            })
            .collect();
        report.sort_by(|a, b| {
            (b.cpu + b.gpu)
                .total_cmp(&(a.cpu + a.gpu))
                .then_with(|| a.id.cmp(&b.id))
        });
        report
    }
}

/// An application's use since the newest sample at least [`WINDOW`] before `at`, or since its
/// processes started.
fn recent(app: &App, at: Duration) -> f64 {
    let Some(&(_, now)) = app.samples.back() else {
        return 0.0;
    };
    if let Some(&(then, before)) = app
        .samples
        .iter()
        .rev()
        .find(|(then, _)| at.saturating_sub(*then) >= WINDOW)
    {
        let seconds = at.saturating_sub(then).as_secs_f64();
        return now.since(before).total() as f64 / 1e9 / seconds;
    }
    // Too new for a window: its running processes' use since they started.
    let used: u64 = app
        .running
        .keys()
        .filter_map(|pid| app.processes.get(pid))
        .map(|usage| usage.total())
        .sum();
    let age = app.running.values().max().copied().unwrap_or_default();
    if age.is_zero() {
        return 0.0;
    }
    used as f64 / 1e9 / age.as_secs_f64()
}

/// An application's use over the last [`AVERAGE`]: since the newest checkpoint at least that old,
/// or, with none, since it began (its use is all its processes' since they started). Samples
/// come with the battery's changes and as the person asks, so the span is the shortest the
/// samples allow that covers twelve hours.
fn average(app: &App, at: Duration) -> f64 {
    let total = app.total();
    if let Some(&(then, before)) = app
        .checkpoints
        .iter()
        .rev()
        .find(|(then, _)| at.saturating_sub(*then) >= AVERAGE)
    {
        let seconds = at.saturating_sub(then).as_secs_f64();
        return total.since(before).total() as f64 / 1e9 / seconds;
    }
    let seconds = app
        .began
        .map_or(0.0, |began| at.saturating_sub(began).as_secs_f64());
    if seconds <= 0.0 {
        return 0.0;
    }
    total.total() as f64 / 1e9 / seconds
}

/// Reads what `running` use now through `proc`.
pub fn read(proc: &Proc<'_>, running: &[Running]) -> Vec<Read> {
    let Some(uptime) = proc.uptime_ns() else {
        return Vec::new();
    };
    running
        .iter()
        .filter_map(|process| {
            let (usage, age) = proc.usage(process.pid, uptime)?;
            Some(Read {
                pid: process.pid,
                usage,
                age,
            })
        })
        .collect()
}
