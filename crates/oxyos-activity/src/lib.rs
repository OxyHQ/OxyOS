//! Activity Monitor's samplers (docs/adr/0063-activity-monitor.mdx): the machine's use and every
//! process's, read from `/proc` as the session's user, two samples making rates. The session asks
//! for a sample every two seconds while Activity Monitor's window is shown and never otherwise
//! (`oxyos_compositor::activity`); nothing here keeps a clock or a thread.
//!
//! - [`Sampler`]: a sample of one view (CPU, Memory, Energy, Disk, Network), each view reading
//!   only what its columns need, since reading every process is the whole cost;
//! - [`inspect`]: one process's details (its files, ports, memory and control group);
//! - [`quit`]: one of the person's own processes ended, through its pidfd.
//!
//! The processor's and the GPU's time are counted as the battery's energy counts them
//! (`oxyos_power::energy`): one way, for both.

pub mod inspect;
mod procfs;
pub mod quit;
mod system;
mod users;

pub use procfs::Procfs;
/// The level of a share of the time some tasks, and all of them, waited for memory: Activity
/// Monitor's graph's, and what the session tells applications (docs/adr/0066-quality-of-service.mdx).
pub use system::level as pressure_level;
pub use system::Level;

use procfs::Stat;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

/// What Activity Monitor shows: each reads what its columns need.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum View {
    Cpu,
    Memory,
    Energy,
    Disk,
    Network,
}

/// An application the session launched that runs: its process, and what is below it, are its.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct App {
    pub id: String,
    pub name: String,
    pub pid: u32,
    /// Its icon's file, as its entry names it.
    pub icon: Option<String>,
}

/// The machine's processors over the interval, in percent of all of them.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Cpu {
    pub user: f64,
    pub system: f64,
    pub idle: f64,
    pub cores: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Pressure {
    /// Percent of the interval in which some tasks stalled for memory.
    pub some: f64,
    /// Percent of it in which all did.
    pub full: f64,
}

/// Memory, in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Memory {
    pub total: u64,
    /// What is not available (`MemTotal` less `MemAvailable`).
    pub used: u64,
    /// The applications' own: anonymous and shared memory.
    pub app: u64,
    pub wired: u64,
    /// What zswap and zram hold, compressed.
    pub compressed: u64,
    /// The files' pages the kernel keeps and may drop.
    pub cached: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    pub pressure: Option<Pressure>,
    pub level: Level,
}

/// What a disk or the network moved: since boot, and per second over the interval.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Disk {
    pub read: u64,
    pub written: u64,
    pub reads: u64,
    pub writes: u64,
    pub read_rate: Option<f64>,
    pub write_rate: Option<f64>,
    pub reads_rate: Option<f64>,
    pub writes_rate: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Network {
    /// Its name; none for every interface together.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub received: u64,
    pub sent: u64,
    pub packets_in: u64,
    pub packets_out: u64,
    pub received_rate: Option<f64>,
    pub sent_rate: Option<f64>,
    pub packets_in_rate: Option<f64>,
    pub packets_out_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interfaces: Option<Vec<Network>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct System {
    pub cpu: Option<Cpu>,
    pub load: [f64; 3],
    pub processes: u32,
    pub threads: u32,
    pub memory: Memory,
    pub disk: Disk,
    pub network: Network,
}

/// What a view adds to a process's row.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Columns {
    Cpu {
        /// Its threads' runs per second.
        wakeups: Option<f64>,
        /// Percent of the GPU's time (its engines' busy time); an application's and the session's.
        gpu: Option<f64>,
        #[serde(rename = "gpuTime")]
        gpu_time: Option<f64>,
    },
    Memory {
        /// Its own: anonymous memory resident and swapped, and the GPU memory it holds.
        memory: u64,
        swap: u64,
        resident: u64,
        #[serde(rename = "gpuMemory")]
        gpu_memory: Option<u64>,
    },
    Energy {
        /// The processor's and the GPU's time over the interval, a hundred for one core busy.
        energy: Option<f64>,
    },
    Disk {
        read: Option<u64>,
        written: Option<u64>,
        #[serde(rename = "readRate")]
        read_rate: Option<f64>,
        #[serde(rename = "writeRate")]
        write_rate: Option<f64>,
    },
    Network {},
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Process {
    pub pid: u32,
    pub ppid: u32,
    /// When it started, in the clock's ticks since boot: with the pid, it names the process.
    pub started: u64,
    pub name: String,
    pub user: String,
    /// The session's user's: one the person may quit and inspect whole.
    pub own: bool,
    /// The application whose process, or below whose, it is.
    pub app: Option<String>,
    /// Percent of one core over the interval.
    pub cpu: Option<f64>,
    /// Seconds.
    pub cpu_time: f64,
    pub threads: u32,
    #[serde(flatten)]
    pub columns: Columns,
}

/// An application's energy: its processes' over the interval, and its average over twelve hours
/// (the battery's meter's, which the session adds).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AppEnergy {
    pub id: String,
    pub name: String,
    pub impact: Option<f64>,
    pub average: Option<f64>,
    pub running: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Sample {
    pub view: View,
    /// Seconds the rates cover: none on the first sample.
    pub interval: f64,
    pub system: System,
    pub apps: Vec<App>,
    pub processes: Vec<Process>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy: Option<Vec<AppEnergy>>,
}

/// What a process's counters were at the last sample.
#[derive(Clone, Copy, Debug, Default)]
struct Counters {
    cpu_ticks: u64,
    runs: Option<u64>,
    gpu_ns: Option<u64>,
    io: Option<(u64, u64)>,
}

#[derive(Clone, Debug)]
struct Before {
    at: Instant,
    processors: Option<system::Processors>,
    pressure: Option<(u64, u64)>,
    disk: system::Moved,
    network: Vec<(String, system::Moved)>,
    processes: HashMap<(u32, u64), Counters>,
}

/// Samples, each against the one before it.
#[derive(Debug)]
pub struct Sampler {
    procfs: Procfs,
    /// The session's user: whose processes are the person's own.
    uid: u32,
    users: users::Users,
    disks: Vec<String>,
    /// Each process's full name, kept while it runs (a cut `comm` costs its command line once).
    names: HashMap<(u32, u64), String>,
    before: Option<Before>,
    buffer: String,
}

/// A rate per second of what grew from `before` to `now` over `seconds`.
fn rate(now: u64, before: u64, seconds: f64) -> Option<f64> {
    (seconds > 0.0).then(|| now.saturating_sub(before) as f64 / seconds)
}

impl Sampler {
    /// A sampler of `procfs`, the processes of `uid` being the person's.
    pub fn new(procfs: Procfs, uid: u32) -> Self {
        let users = users::Users::new(procfs.passwd.clone());
        let disks = system::disks(&procfs);
        Self {
            procfs,
            uid,
            users,
            disks,
            names: HashMap::new(),
            before: None,
            buffer: String::with_capacity(4096),
        }
    }

    /// The system's, the session's user being the person.
    pub fn system() -> Self {
        Self::new(Procfs::system(), rustix::process::getuid().as_raw())
    }

    pub fn procfs(&self) -> &Procfs {
        &self.procfs
    }

    /// Forgets the last sample: the next has no rates. Following begins again after the window
    /// was hidden; the disks are looked for again (one plugged in since).
    pub fn reset(&mut self) {
        self.before = None;
        self.disks = system::disks(&self.procfs);
    }

    /// A sample of `view` at `now`. `apps` are the applications the session launched that run;
    /// `gpu` are processes beyond theirs whose GPU is read (the session's own).
    pub fn sample(&mut self, view: View, now: Instant, apps: &[App], gpu: &[u32]) -> Sample {
        self.users.begin();
        let before = self.before.take();
        let seconds = before
            .as_ref()
            .map_or(0.0, |before| now.duration_since(before.at).as_secs_f64());
        let ticks = self.procfs.ticks.max(1) as f64;

        // Every process's stat: what every view needs.
        let mut stats: Vec<Stat> = Vec::new();
        for pid in self.procfs.pids() {
            if let Some(stat) = procfs::stat(&self.procfs, pid, &mut self.buffer) {
                stats.push(stat);
            }
        }
        // Each application's processes: its launched one and every one below it.
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        for stat in &stats {
            children.entry(stat.ppid).or_default().push(stat.pid);
        }
        let mut owner: HashMap<u32, usize> = HashMap::new();
        for (index, app) in apps.iter().enumerate() {
            let mut queue = vec![app.pid];
            while let Some(pid) = queue.pop() {
                if owner.insert(pid, index).is_none() {
                    queue.extend(children.get(&pid).into_iter().flatten());
                }
            }
        }
        let gpu: HashSet<u32> = gpu.iter().copied().collect();

        let mut counters = HashMap::with_capacity(stats.len());
        let mut processes = Vec::with_capacity(stats.len());
        let mut threads = 0;
        for stat in &stats {
            threads += stat.threads;
            let key = (stat.pid, stat.started);
            let was = before.as_ref().and_then(|b| b.processes.get(&key)).copied();
            let mut now_counters = Counters {
                cpu_ticks: stat.cpu_ticks,
                ..Counters::default()
            };
            let app = owner.get(&stat.pid).map(|index| &apps[*index]);
            let own = stat.uid == self.uid;
            let name = match app.filter(|app| app.pid == stat.pid) {
                Some(app) => app.name.clone(),
                None => self
                    .names
                    .entry(key)
                    .or_insert_with(|| procfs::full_name(&self.procfs, stat.pid, &stat.comm))
                    .clone(),
            };
            let per_second =
                |now: u64, then: Option<u64>| then.and_then(|then| rate(now, then, seconds));
            let cpu = was.and_then(|was| {
                rate(stat.cpu_ticks, was.cpu_ticks, seconds).map(|t| t / ticks * 100.0)
            });
            let reads_gpu = app.is_some() || gpu.contains(&stat.pid);
            let mut drm = None;
            if reads_gpu && matches!(view, View::Cpu | View::Energy | View::Memory) {
                drm = Some(self.procfs.energy().drm(stat.pid, &mut HashSet::new()));
                now_counters.gpu_ns = drm.map(|d| d.busy_ns);
            }
            let gpu_share = now_counters
                .gpu_ns
                .and_then(|ns| per_second(ns, was.and_then(|w| w.gpu_ns)))
                .map(|ns| ns / 1e9 * 100.0);
            let columns = match view {
                View::Cpu => {
                    now_counters.runs = procfs::runs(&self.procfs, stat.pid, &mut self.buffer);
                    Columns::Cpu {
                        wakeups: now_counters
                            .runs
                            .and_then(|runs| per_second(runs, was.and_then(|w| w.runs))),
                        gpu: gpu_share,
                        gpu_time: now_counters.gpu_ns.map(|ns| ns as f64 / 1e9),
                    }
                }
                View::Memory => {
                    let status = procfs::status(&self.procfs, stat.pid, &mut self.buffer)
                        .unwrap_or_default();
                    let gpu_memory = drm.map(|d| d.memory);
                    Columns::Memory {
                        memory: status.anonymous + status.swap + gpu_memory.unwrap_or(0),
                        swap: status.swap,
                        resident: status.resident,
                        gpu_memory,
                    }
                }
                View::Energy => {
                    let cpu_ns = was
                        .map(|w| stat.cpu_ticks.saturating_sub(w.cpu_ticks) as f64 / ticks * 1e9);
                    let gpu_ns = now_counters.gpu_ns.map(|ns| {
                        was.and_then(|w| w.gpu_ns)
                            .map(|then| ns.saturating_sub(then))
                    });
                    let energy = match (cpu_ns, gpu_ns) {
                        (Some(cpu), Some(Some(gpu))) => Some(cpu + gpu as f64),
                        (Some(cpu), None) => Some(cpu),
                        _ => None,
                    }
                    .filter(|_| seconds > 0.0)
                    .map(|ns| ns / 1e9 / seconds * 100.0);
                    Columns::Energy { energy }
                }
                View::Disk => {
                    now_counters.io = if own {
                        procfs::io(&self.procfs, stat.pid, &mut self.buffer)
                    } else {
                        None
                    };
                    let then = was.and_then(|w| w.io);
                    Columns::Disk {
                        read: now_counters.io.map(|io| io.0),
                        written: now_counters.io.map(|io| io.1),
                        read_rate: now_counters
                            .io
                            .and_then(|io| per_second(io.0, then.map(|t| t.0))),
                        write_rate: now_counters
                            .io
                            .and_then(|io| per_second(io.1, then.map(|t| t.1))),
                    }
                }
                View::Network => Columns::Network {},
            };
            counters.insert(key, now_counters);
            processes.push(Process {
                pid: stat.pid,
                ppid: stat.ppid,
                started: stat.started,
                name,
                user: self.users.name(stat.uid),
                own,
                app: app.map(|app| app.id.clone()),
                cpu,
                cpu_time: stat.cpu_ticks as f64 / ticks,
                threads: stat.threads,
                columns,
            });
        }
        self.names.retain(|key, _| counters.contains_key(key));

        let (system, mut before_now) = self.machine(before.as_ref(), seconds, now);
        before_now.processes = counters;
        let system = System {
            processes: processes.len() as u32,
            threads,
            ..system
        };
        let energy = (view == View::Energy).then(|| {
            apps.iter()
                .map(|app| {
                    let mut impacts = processes
                        .iter()
                        .filter(|p| p.app.as_deref() == Some(app.id.as_str()))
                        .filter_map(|p| match p.columns {
                            Columns::Energy { energy } => energy,
                            _ => None,
                        })
                        .peekable();
                    let impact = impacts.peek().is_some().then(|| impacts.sum());
                    AppEnergy {
                        id: app.id.clone(),
                        name: app.name.clone(),
                        impact,
                        average: None,
                        running: true,
                    }
                })
                .collect()
        });
        self.before = Some(before_now);
        Sample {
            view,
            interval: seconds,
            system,
            apps: apps.to_vec(),
            processes,
            energy,
        }
    }

    /// The machine's use, and its counters for the next sample.
    fn machine(&mut self, before: Option<&Before>, seconds: f64, now: Instant) -> (System, Before) {
        let buffer = &mut self.buffer;
        let read = |name: &str, buffer: &mut String| {
            procfs::read_into(&self.procfs.proc.join(name), buffer).is_some()
        };
        let processors = read("stat", buffer)
            .then(|| system::processors(buffer))
            .flatten();
        let cpu = match (processors, before.and_then(|b| b.processors)) {
            (Some(now), Some(then)) => {
                let user = now.user.saturating_sub(then.user) as f64;
                let system = now.system.saturating_sub(then.system) as f64;
                let idle = now.idle.saturating_sub(then.idle) as f64;
                let all = user + system + idle;
                (all > 0.0).then(|| Cpu {
                    user: user / all * 100.0,
                    system: system / all * 100.0,
                    idle: idle / all * 100.0,
                    cores: now.cores,
                })
            }
            _ => None,
        };
        let mut load = [0.0; 3];
        if read("loadavg", buffer) {
            for (slot, value) in load.iter_mut().zip(buffer.split_whitespace()) {
                *slot = value.parse().unwrap_or(0.0);
            }
        }
        let info = if read("meminfo", buffer) {
            system::meminfo(buffer)
        } else {
            system::Meminfo::default()
        };
        let stalled = read("pressure/memory", buffer)
            .then(|| system::pressure(buffer))
            .flatten();
        let pressure = match (stalled, before.and_then(|b| b.pressure)) {
            (Some(now), Some(then)) if seconds > 0.0 => {
                let share = |now: u64, then: u64| {
                    (now.saturating_sub(then) as f64 / 1e6 / seconds * 100.0).min(100.0)
                };
                Some(Pressure {
                    some: share(now.0, then.0),
                    full: share(now.1, then.1),
                })
            }
            _ => None,
        };
        let compressed = info.zswap + system::zram(&self.procfs, buffer);
        let memory = Memory {
            total: info.total,
            used: info.total.saturating_sub(info.available),
            app: info.anonymous + info.shmem,
            wired: info.wired,
            compressed,
            cached: (info.cached + info.buffers).saturating_sub(info.shmem),
            swap_used: info.swap_total.saturating_sub(info.swap_free),
            swap_total: info.swap_total,
            pressure,
            level: pressure.map_or(Level::Normal, |p| system::level(p.some, p.full)),
        };
        let moved = if read("diskstats", buffer) {
            system::diskstats(buffer, &self.disks)
        } else {
            system::Moved::default()
        };
        let then = before.map(|b| b.disk);
        let per = |now: u64, then: Option<u64>| then.and_then(|then| rate(now, then, seconds));
        let disk = Disk {
            read: moved.bytes_in,
            written: moved.bytes_out,
            reads: moved.count_in,
            writes: moved.count_out,
            read_rate: per(moved.bytes_in, then.map(|t| t.bytes_in)),
            write_rate: per(moved.bytes_out, then.map(|t| t.bytes_out)),
            reads_rate: per(moved.count_in, then.map(|t| t.count_in)),
            writes_rate: per(moved.count_out, then.map(|t| t.count_out)),
        };
        let interfaces = if read("net/dev", buffer) {
            system::interfaces(buffer)
        } else {
            Vec::new()
        };
        let network_of =
            |name: Option<String>, moved: system::Moved, then: Option<system::Moved>| Network {
                name,
                received: moved.bytes_in,
                sent: moved.bytes_out,
                packets_in: moved.count_in,
                packets_out: moved.count_out,
                received_rate: per(moved.bytes_in, then.map(|t| t.bytes_in)),
                sent_rate: per(moved.bytes_out, then.map(|t| t.bytes_out)),
                packets_in_rate: per(moved.count_in, then.map(|t| t.count_in)),
                packets_out_rate: per(moved.count_out, then.map(|t| t.count_out)),
                interfaces: None,
            };
        let mut total = system::Moved::default();
        let mut total_then = system::Moved::default();
        let mut all_known = before.is_some();
        let each: Vec<Network> = interfaces
            .iter()
            .map(|(name, moved)| {
                let then = before.and_then(|b| {
                    b.network
                        .iter()
                        .find(|(was, _)| was == name)
                        .map(|(_, m)| *m)
                });
                total = system::Moved {
                    bytes_in: total.bytes_in + moved.bytes_in,
                    bytes_out: total.bytes_out + moved.bytes_out,
                    count_in: total.count_in + moved.count_in,
                    count_out: total.count_out + moved.count_out,
                };
                match then {
                    Some(t) => {
                        total_then = system::Moved {
                            bytes_in: total_then.bytes_in + t.bytes_in,
                            bytes_out: total_then.bytes_out + t.bytes_out,
                            count_in: total_then.count_in + t.count_in,
                            count_out: total_then.count_out + t.count_out,
                        }
                    }
                    // An interface that came up since: no rate for the whole either.
                    None => all_known = false,
                }
                network_of(Some(name.clone()), *moved, then)
            })
            .collect();
        let mut network = network_of(None, total, all_known.then_some(total_then));
        network.interfaces = Some(each);
        (
            System {
                cpu,
                load,
                processes: 0,
                threads: 0,
                memory,
                disk,
                network,
            },
            Before {
                at: now,
                processors,
                pressure: stalled,
                disk: moved,
                network: interfaces,
                processes: HashMap::new(),
            },
        )
    }
}

#[cfg(test)]
mod tests;
