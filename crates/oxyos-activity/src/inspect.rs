//! One process's details, as a Mac's *Inspect* sheet shows them: its parent, user and times; its
//! memory counted page by page (`smaps_rollup`, which costs too much to read for every process
//! every two seconds, docs/adr/0060-activity-monitor.mdx, decision 4); the files it has open and
//! its sockets (`/proc/PID/fd`, each socket found in its network namespace's tables); and its
//! control group. Read once, when asked. What only the process's own user may read (its files,
//! sockets, path, arguments and memory) is left out of another user's, which says so
//! (`restricted`).

use crate::procfs::{self, Procfs};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

/// At most this many files and ports, and arguments, each argument cut at its byte limit: an
/// answer stays well within a system answer's size.
const MAX_FILES: usize = 512;
const MAX_PORTS: usize = 256;
const MAX_ARGUMENTS: usize = 64;
const MAX_ARGUMENT_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Memory {
    pub resident: u64,
    /// Its unique memory and the GPU's it holds: perf.py's footprint.
    pub footprint: u64,
    /// Pages only it maps (`Private_Clean` and `Private_Dirty`).
    pub unique: u64,
    /// Its share of every page it maps.
    pub proportional: u64,
    pub swap: u64,
    pub gpu: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct File {
    pub fd: u32,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Port {
    pub fd: u32,
    pub kind: &'static str,
    pub local: String,
    pub remote: Option<String>,
    pub state: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inspect {
    pub pid: u32,
    pub started: u64,
    pub name: String,
    pub path: Option<String>,
    pub arguments: Option<Vec<String>>,
    pub ppid: u32,
    pub parent_name: Option<String>,
    pub user: String,
    pub own: bool,
    pub app: Option<String>,
    /// Seconds since the epoch.
    pub started_at: u64,
    pub cpu_time: f64,
    pub threads: u32,
    pub memory: Option<Memory>,
    pub files: Option<Vec<File>>,
    pub ports: Option<Vec<Port>>,
    pub cgroup: Option<String>,
    pub restricted: bool,
}

/// The process `pid` that started at `started`, or none once it is gone (or its pid is
/// another's). `uid` is the session's user; `app` the application it is of, as the session knows.
pub fn inspect(
    procfs: &Procfs,
    pid: u32,
    started: u64,
    uid: u32,
    app: Option<String>,
) -> Option<Inspect> {
    let mut buffer = String::new();
    let stat = procfs::stat(procfs, pid, &mut buffer).filter(|s| s.started == started)?;
    let status = procfs::status(procfs, pid, &mut buffer).unwrap_or_default();
    let own = status.uids.iter().all(|id| *id == uid);
    let ticks = procfs.ticks.max(1);
    let boot = procfs::read_into(&procfs.proc.join("stat"), &mut buffer)
        .and_then(|_| crate::system::boot_time(&buffer))
        .unwrap_or(0);
    let parent_name = procfs::stat(procfs, stat.ppid, &mut buffer)
        .map(|parent| procfs::full_name(procfs, parent.pid, &parent.comm));
    let cgroup = procfs::read_into(&procfs.of(pid, "cgroup"), &mut buffer).and_then(|_| {
        buffer
            .lines()
            .find_map(|line| line.strip_prefix("0::"))
            .map(str::to_owned)
    });
    let users = crate::users::Users::new(procfs.passwd.clone());
    let mut users = users;
    let mut inspected = Inspect {
        pid,
        started,
        name: procfs::full_name(procfs, pid, &stat.comm),
        path: None,
        arguments: None,
        ppid: stat.ppid,
        parent_name,
        user: users.name(status.uids[1]),
        own,
        app,
        started_at: boot + started / ticks,
        cpu_time: stat.cpu_ticks as f64 / ticks as f64,
        threads: stat.threads,
        memory: None,
        files: None,
        ports: None,
        cgroup,
        restricted: !own,
    };
    if !own {
        return Some(inspected);
    }
    inspected.path = std::fs::read_link(procfs.of(pid, "exe"))
        .ok()
        .map(|path| path.to_string_lossy().into_owned());
    inspected.arguments = std::fs::read(procfs.of(pid, "cmdline")).ok().map(|bytes| {
        bytes
            .split(|b| *b == 0)
            .filter(|argument| !argument.is_empty())
            .take(MAX_ARGUMENTS)
            .map(|argument| {
                let cut = &argument[..argument.len().min(MAX_ARGUMENT_BYTES)];
                String::from_utf8_lossy(cut).into_owned()
            })
            .collect()
    });
    inspected.memory = procfs::read_into(&procfs.of(pid, "smaps_rollup"), &mut buffer).map(|_| {
        let mut memory = smaps_rollup(&buffer);
        memory.gpu = procfs.energy().drm(pid, &mut HashSet::new()).memory;
        memory.footprint = memory.unique + memory.gpu;
        memory
    });
    let (files, ports) = descriptors(procfs, pid);
    inspected.files = Some(files);
    inspected.ports = Some(ports);
    Some(inspected)
}

/// `smaps_rollup`'s totals, in bytes.
pub fn smaps_rollup(text: &str) -> Memory {
    let mut memory = Memory::default();
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
            "Rss" => memory.resident = bytes,
            "Pss" => memory.proportional = bytes,
            "Private_Clean" | "Private_Dirty" => memory.unique += bytes,
            "Swap" => memory.swap = bytes,
            _ => {}
        }
    }
    memory
}

/// The files a process has open (regular files, directories, devices) and its sockets, each by
/// its descriptor in order.
fn descriptors(procfs: &Procfs, pid: u32) -> (Vec<File>, Vec<Port>) {
    let mut open: Vec<(u32, String)> = std::fs::read_dir(procfs.of(pid, "fd"))
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| {
                    let fd = entry.file_name().to_str()?.parse().ok()?;
                    let target = std::fs::read_link(entry.path()).ok()?;
                    Some((fd, target.to_string_lossy().into_owned()))
                })
                .collect()
        })
        .unwrap_or_default();
    open.sort_by_key(|(fd, _)| *fd);
    let files = open
        .iter()
        .filter(|(_, target)| target.starts_with('/'))
        .take(MAX_FILES)
        .map(|(fd, path)| File {
            fd: *fd,
            path: path.clone(),
        })
        .collect();
    let inodes: Vec<(u32, u64)> = open
        .iter()
        .filter_map(|(fd, target)| {
            let inode = target.strip_prefix("socket:[")?.strip_suffix(']')?;
            Some((*fd, inode.parse().ok()?))
        })
        .collect();
    let mut ports = Vec::new();
    if !inodes.is_empty() {
        let sockets = sockets(procfs, pid);
        for (fd, inode) in inodes {
            if ports.len() == MAX_PORTS {
                break;
            }
            if let Some(socket) = sockets.get(&inode) {
                ports.push(Port {
                    fd,
                    kind: socket.0,
                    local: socket.1.clone(),
                    remote: socket.2.clone(),
                    state: socket.3,
                });
            }
        }
    }
    (files, ports)
}

type Socket = (&'static str, String, Option<String>, Option<&'static str>);

/// The sockets of the process's network namespace by inode: its TCP and UDP ones (address, peer,
/// state) and its Unix ones (their path, or none).
fn sockets(procfs: &Procfs, pid: u32) -> HashMap<u64, Socket> {
    let mut found = HashMap::new();
    for kind in ["tcp", "tcp6", "udp", "udp6"] {
        let Ok(text) = std::fs::read_to_string(procfs.of(pid, &format!("net/{kind}"))) else {
            continue;
        };
        for line in text.lines().skip(1) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let (Some(local), Some(remote), Some(state), Some(inode)) = (
                fields.get(1).and_then(|a| address(a)),
                fields.get(2).and_then(|a| address(a)),
                fields.get(3),
                fields.get(9).and_then(|i| i.parse::<u64>().ok()),
            ) else {
                continue;
            };
            let tcp = kind.starts_with("tcp");
            let unconnected = remote.ends_with(":0");
            found.insert(
                inode,
                (
                    kind_name(kind),
                    local,
                    (!unconnected).then_some(remote),
                    if tcp { tcp_state(state) } else { None },
                ),
            );
        }
    }
    if let Ok(text) = std::fs::read_to_string(procfs.of(pid, "net/unix")) {
        for line in text.lines().skip(1) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let Some(inode) = fields.get(6).and_then(|i| i.parse::<u64>().ok()) else {
                continue;
            };
            let path = fields.get(7).map(|p| (*p).to_owned()).unwrap_or_default();
            found.insert(inode, ("unix", path, None, None));
        }
    }
    found
}

fn kind_name(kind: &str) -> &'static str {
    match kind {
        "tcp" => "tcp",
        "tcp6" => "tcp6",
        "udp" => "udp",
        _ => "udp6",
    }
}

/// The kernel's TCP states (`include/net/tcp_states.h`).
fn tcp_state(code: &str) -> Option<&'static str> {
    Some(match u8::from_str_radix(code, 16).ok()? {
        1 => "established",
        2 => "synSent",
        3 => "synReceived",
        4 => "finWait1",
        5 => "finWait2",
        6 => "timeWait",
        7 => "closed",
        8 => "closeWait",
        9 => "lastAck",
        10 => "listen",
        11 => "closing",
        _ => return None,
    })
}

/// A `/proc/net` address (`0100007F:0277`, or 32 hexadecimal digits for IPv6), as
/// `127.0.0.1:631` or `[::1]:631`. The kernel writes each 32-bit word in the machine's order.
pub fn address(text: &str) -> Option<String> {
    let (host, port) = text.split_once(':')?;
    let port = u16::from_str_radix(port, 16).ok()?;
    let words: Option<Vec<u32>> = (0..host.len() / 8)
        .map(|i| u32::from_str_radix(host.get(i * 8..i * 8 + 8)?, 16).ok())
        .collect();
    let bytes: Vec<u8> = words?
        .into_iter()
        .flat_map(|word| word.swap_bytes().to_be_bytes())
        .collect();
    match bytes.len() {
        4 => Some(format!(
            "{}:{port}",
            std::net::Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3])
        )),
        16 => {
            let octets: [u8; 16] = bytes.try_into().ok()?;
            let ip = std::net::Ipv6Addr::from(octets);
            Some(match ip.to_ipv4_mapped() {
                Some(v4) => format!("{v4}:{port}"),
                None => format!("[{ip}]:{port}"),
            })
        }
        _ => None,
    }
}
