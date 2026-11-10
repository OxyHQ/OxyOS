//! Loads for the saturation benchmark (`perf.py saturation`, docs/performance.mdx, docs/adr/0066):
//! what keeps a machine busy while the input measured under it is given, run as an application
//! the session launches (a development entry's `Exec`) or as another user's or a service's
//! process the benchmark places itself. Never installed.
//!
//! `oxyos-offscreen --load KIND ... [--progress FILE]`:
//! - `spin THREADS`: THREADS threads computing without rest, in one process (one application's
//!   many threads);
//! - `processes N`: N processes of one thread each (a build's jobs, `stress-ng --cpu N`), ended
//!   with this one;
//! - `memory MIB`: MIB of memory taken a chunk at a time and then written again and again, a page
//!   at a time, so every page stays in use (a memory hog: reclaim, swap, thrashing);
//! - `repeat -- COMMAND ARGS...`: COMMAND run again each time it ends (a build started over), in
//!   a process group of its own that ends with this process.
//!
//! With `--progress FILE` it writes, twice a second, one JSON line in place of the last:
//! `{"work": N, "cpu_ms": M}`, the work done so far (iterations in millions, pages written, runs
//! completed) and the CPU time its processes used, so the benchmark sees that a load yields
//! rather than stops.

use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const USAGE: &str = "usage: oxyos-offscreen --load spin THREADS | processes N | memory MIB | \
                     repeat -- COMMAND [ARGS...] [--progress FILE] [--parent PID]";

/// A page, as the kernel keeps memory.
const PAGE: usize = 4096;

/// What a load is asked to do.
#[derive(Debug, PartialEq, Eq)]
enum Kind {
    Spin(u32),
    Processes(u32),
    Memory(u64),
    Repeat(Vec<OsString>),
}

/// A load's arguments: its kind, where it says how far it got, and the process it ends with.
#[derive(Debug, PartialEq, Eq)]
struct Load {
    kind: Kind,
    progress: Option<PathBuf>,
    parent: Option<u32>,
}

fn parse(args: &[OsString]) -> Result<Load> {
    let mut args = args.iter();
    let number = |value: Option<&OsString>| -> Result<u64> {
        Ok(value
            .and_then(|value| value.to_str())
            .ok_or(USAGE)?
            .parse::<u64>()
            .map_err(|_| USAGE)?)
    };
    let kind = match args.next().and_then(|kind| kind.to_str()) {
        Some("spin") => Kind::Spin(u32::try_from(number(args.next())?)?),
        Some("processes") => Kind::Processes(u32::try_from(number(args.next())?)?),
        Some("memory") => Kind::Memory(number(args.next())?),
        Some("repeat") => Kind::Repeat(Vec::new()),
        _ => return Err(USAGE.into()),
    };
    let (mut progress, mut parent, mut command) = (None, None, Vec::new());
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--progress") => progress = Some(PathBuf::from(args.next().ok_or(USAGE)?)),
            Some("--parent") => parent = Some(u32::try_from(number(args.next())?)?),
            Some("--") => {
                command = args.by_ref().cloned().collect();
                break;
            }
            _ => return Err(format!("unexpected argument {arg:?}\n{USAGE}").into()),
        }
    }
    let kind = match kind {
        Kind::Repeat(_) if command.is_empty() => return Err("repeat needs -- COMMAND".into()),
        Kind::Repeat(_) => Kind::Repeat(command),
        _ if !command.is_empty() => return Err(USAGE.into()),
        Kind::Spin(0) | Kind::Processes(0) | Kind::Memory(0) => {
            return Err("a load of nothing".into())
        }
        kind => kind,
    };
    Ok(Load {
        kind,
        progress,
        parent,
    })
}

/// Runs the load `args` describes (after `--load`) until it is ended.
pub fn main(args: &[OsString]) -> Result<()> {
    let load = parse(args)?;
    // Ended with the process that started it, if it says which: a job of `processes`.
    if let Some(parent) = load.parent {
        rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::KILL))?;
        if rustix::process::getppid().map(|pid| pid.as_raw_nonzero().get() as u32) != Some(parent) {
            return Ok(());
        }
    }
    let work = Arc::new(AtomicU64::new(0));
    let children: Arc<std::sync::Mutex<Vec<u32>>> = Arc::default();
    if let Some(file) = load.progress.clone() {
        let (work, children) = (work.clone(), children.clone());
        std::thread::Builder::new()
            .name("load-progress".into())
            .spawn(move || loop {
                let children = children.lock().map(|pids| pids.clone()).unwrap_or_default();
                let line = format!(
                    "{{\"work\":{},\"cpu_ms\":{}}}\n",
                    work.load(Ordering::Relaxed),
                    cpu_ms(&children)
                );
                let staged = file.with_extension("staged");
                if std::fs::write(&staged, line).is_ok() {
                    let _ = std::fs::rename(&staged, &file);
                }
                std::thread::sleep(Duration::from_millis(500));
            })?;
    }
    match load.kind {
        Kind::Spin(threads) => spin(threads, &work),
        Kind::Processes(n) => processes(n, &children),
        Kind::Memory(mib) => memory(mib, &work),
        Kind::Repeat(command) => {
            let stop = Arc::new(AtomicBool::new(false));
            for signal in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT] {
                signal_hook::flag::register(signal, stop.clone())?;
            }
            repeat(&command, &work, &children, &stop)
        }
    }
}

/// A process's CPU time in clock ticks (USER_HZ, 100 a second on Linux) and its process group, from
/// `/proc/PID/stat`: every thread's, and its children's it waited for (a build's compilers).
fn ticks(pid: &str) -> Option<(u64, u32)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    // After the name: state, ppid, pgrp, ..., utime (14th field), stime, cutime, cstime.
    let group = fields.get(2)?.parse().ok()?;
    let used = fields
        .get(11..15)?
        .iter()
        .filter_map(|field| field.parse::<u64>().ok())
        .sum();
    Some((used, group))
}

/// The CPU this load used, in milliseconds: this process's, and that of every process of the
/// groups or processes it started (`started`: a job's pid, or a repeated command's group).
fn cpu_ms(started: &[u32]) -> u64 {
    let mut total = ticks("self").map_or(0, |(used, _)| used);
    if !started.is_empty() {
        for entry in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
            let name = entry.file_name();
            let Some(pid) = name
                .to_str()
                .filter(|n| n.bytes().all(|b| b.is_ascii_digit()))
            else {
                continue;
            };
            if let Some((used, group)) = ticks(pid) {
                let started_it = pid.parse().is_ok_and(|pid: u32| started.contains(&pid));
                if started_it || started.contains(&group) {
                    total += used;
                }
            }
        }
    }
    total * 10
}

/// One thread's computing without rest: a million steps of a xorshift at a time, counted.
fn compute(work: &AtomicU64) -> ! {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64 ^ u64::from(std::process::id());
    loop {
        for _ in 0..1_000_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state = std::hint::black_box(state);
        }
        work.fetch_add(1, Ordering::Relaxed);
    }
}

fn spin(threads: u32, work: &Arc<AtomicU64>) -> Result<()> {
    for index in 1..threads {
        let work = work.clone();
        std::thread::Builder::new()
            .name(format!("load-spin-{index}"))
            .spawn(move || compute(&work))?;
    }
    compute(work)
}

fn processes(n: u32, children: &Arc<std::sync::Mutex<Vec<u32>>>) -> Result<()> {
    let me = std::process::id().to_string();
    let mut started = Vec::new();
    for _ in 0..n {
        let child = std::process::Command::new(std::env::current_exe()?)
            .args(["--load", "spin", "1", "--parent", &me])
            .spawn()?;
        children.lock().map_err(|_| "poisoned")?.push(child.id());
        started.push(child);
    }
    for mut child in started {
        child.wait()?;
    }
    Ok(())
}

fn memory(mib: u64, work: &AtomicU64) -> Result<()> {
    const CHUNK: usize = 64 << 20;
    let total = usize::try_from(mib << 20)?;
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    let mut taken = 0;
    // Taken a chunk at a time, each page written as it is taken (an untouched page is no memory).
    while taken < total {
        let size = CHUNK.min(total - taken);
        let mut chunk = vec![0u8; size];
        for page in chunk.iter_mut().step_by(PAGE) {
            *page = 1;
        }
        work.fetch_add((size / PAGE) as u64, Ordering::Relaxed);
        chunks.push(chunk);
        taken += size;
    }
    // Then every page written again, over and over: all of it in use, none of it idle to reclaim.
    let mut round = 1u8;
    loop {
        round = round.wrapping_add(1);
        for chunk in &mut chunks {
            for page in chunk.iter_mut().step_by(PAGE) {
                *page = round;
            }
            work.fetch_add((chunk.len() / PAGE) as u64, Ordering::Relaxed);
        }
    }
}

fn repeat(
    command: &[OsString],
    work: &AtomicU64,
    children: &Arc<std::sync::Mutex<Vec<u32>>>,
    stop: &AtomicBool,
) -> Result<()> {
    use std::os::unix::process::CommandExt;
    while !stop.load(Ordering::Relaxed) {
        // A group of its own, so what it starts (a build's compilers) ends with it.
        let mut child = std::process::Command::new(&command[0])
            .args(&command[1..])
            .process_group(0)
            .spawn()?;
        let group = child.id();
        children.lock().map_err(|_| "poisoned")?.push(group);
        loop {
            if stop.load(Ordering::Relaxed) {
                end_group(group);
                let _ = child.wait();
                return Ok(());
            }
            match child.try_wait()? {
                Some(status) => {
                    if !status.success() {
                        eprintln!("[load] {command:?} ended {status}; again");
                    }
                    work.fetch_add(1, Ordering::Relaxed);
                    break;
                }
                None => std::thread::sleep(Duration::from_millis(50)),
            }
        }
        // What the run left in its group (a build's jobs), ended before the next.
        end_group(group);
        children
            .lock()
            .map_err(|_| "poisoned")?
            .retain(|pid| *pid != group);
    }
    Ok(())
}

fn end_group(group: u32) {
    let pid = i32::try_from(group)
        .ok()
        .and_then(rustix::process::Pid::from_raw);
    if let Some(pid) = pid {
        match rustix::process::kill_process_group(pid, rustix::process::Signal::KILL) {
            Ok(()) => {}
            Err(error) if error == rustix::io::Errno::SRCH => {}
            Err(error) => eprintln!("[load] its group {group}: {}", io::Error::from(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> Vec<OsString> {
        text.split(' ').map(OsString::from).collect()
    }

    #[test]
    fn kinds_and_their_arguments() {
        assert_eq!(
            parse(&args("spin 8 --progress /p")).unwrap(),
            Load {
                kind: Kind::Spin(8),
                progress: Some(PathBuf::from("/p")),
                parent: None
            }
        );
        assert_eq!(
            parse(&args("processes 4")).unwrap().kind,
            Kind::Processes(4)
        );
        assert_eq!(
            parse(&args("memory 512 --parent 7")).unwrap(),
            Load {
                kind: Kind::Memory(512),
                progress: None,
                parent: Some(7)
            }
        );
        assert_eq!(
            parse(&args("repeat --progress /p -- cargo build -j 4"))
                .unwrap()
                .kind,
            Kind::Repeat(args("cargo build -j 4"))
        );
    }

    #[test]
    fn nothing_to_do_is_refused() {
        for wrong in [
            "spin 0",
            "memory 0",
            "repeat",
            "repeat --",
            "spin",
            "fly 3",
            "spin 2 -- x",
        ] {
            assert!(parse(&args(wrong)).is_err(), "{wrong} was taken");
        }
    }

    #[test]
    fn a_repeated_command_counts_its_runs_and_ends_its_group() {
        let dir = std::env::temp_dir().join(format!("oxyos-load-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let work = Arc::new(AtomicU64::new(0));
        let children: Arc<std::sync::Mutex<Vec<u32>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (w, c, s) = (work.clone(), children.clone(), stop.clone());
        let marker = dir.join("ran");
        let script = format!("echo x >> {}", marker.display());
        let command: Vec<OsString> = vec!["sh".into(), "-c".into(), script.into()];
        let running = std::thread::spawn(move || repeat(&command, &w, &c, &s).is_ok());
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while work.load(Ordering::Relaxed) < 3 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(work.load(Ordering::Relaxed) >= 3);
        let runs = std::fs::read_to_string(&marker).unwrap().lines().count();
        assert!(runs >= 3, "{runs} runs");
        stop.store(true, Ordering::Relaxed);
        assert!(running.join().unwrap(), "it ended in error");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
