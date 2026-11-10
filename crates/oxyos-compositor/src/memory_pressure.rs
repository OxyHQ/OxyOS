//! Memory pressure (docs/adr/0066-quality-of-service.mdx, decision 7): the session follows the
//! kernel's pressure stall information and tells applications how short of memory the machine is
//! (the scene's `set_memory_pressure`), so they give back what they can make again before the
//! kernel must take it, as iOS's memory warnings and Android's `onTrimMemory`.
//!
//! What it follows: the machine's (`/proc/pressure/memory`) and the applications' (the user
//! manager's `app.slice`, its `memory.pressure`, where it has one). On each, triggers (`poll`'s
//! `POLLPRI`) at the levels Activity Monitor's graph colours (docs/adr/0063-activity-monitor.mdx,
//! decision 4): a warning when some tasks wait for memory a tenth of a 2 s window, critical at 40 %
//! some or a tenth all, the windows a multiple of 2 s as an unprivileged trigger must be. A
//! trigger raises the level at once; it comes down only as the ten-second averages say, read every
//! 2 s while it is up, so a level is never dropped and raised again on the edge of a threshold.
//! Its thread waits on the triggers alone while memory is plentiful: no timer.
//!
//! The development compositor follows the files its environment names instead: the machine's
//! (`OXYOS_MEMORY_PRESSURE_FIXTURE`, the saturation benchmark's machine, a control group) and the
//! applications' under its user manager's (`OXYOS_CGROUP_FIXTURE`).

use crate::stop::Stop;
use oxyos_compositor_core::Scene;
use oxyos_protocol::MemoryPressureLevel;
use rustix::event::{poll, PollFd, PollFlags, Timespec};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;

/// The machine's pressure, as the kernel says it to any user.
pub const MACHINE: &str = "/proc/pressure/memory";

/// The triggers armed on each file: the stall, in microseconds of a window, at which it fires,
/// and the level it says. The thresholds are `oxyos_activity::pressure_level`'s.
const TRIGGERS: [(&str, MemoryPressureLevel); 3] = [
    ("some 200000 2000000", MemoryPressureLevel::Warning),
    ("some 800000 2000000", MemoryPressureLevel::Critical),
    ("full 200000 2000000", MemoryPressureLevel::Critical),
];

/// While the level is up, the averages are read this often to see it come down.
const EASING: Duration = Duration::from_secs(2);

/// The files the session follows: the machine's, and its applications' where its user manager has
/// an `app.slice` (`/proc/self/cgroup`'s session scope's sibling `user@UID.service`).
pub fn session_sources() -> Vec<PathBuf> {
    let mut sources = vec![std::env::var_os("OXYOS_MEMORY_PRESSURE_FIXTURE")
        .map_or_else(|| PathBuf::from(MACHINE), PathBuf::from)];
    let applications = match std::env::var_os("OXYOS_CGROUP_FIXTURE") {
        Some(manager) => Some(PathBuf::from(manager).join("app.slice")),
        None => std::fs::read_to_string("/proc/self/cgroup")
            .ok()
            .zip(std::fs::metadata("/proc/self").ok())
            .and_then(|(text, me)| user_manager(&text, std::os::unix::fs::MetadataExt::uid(&me)))
            .map(|manager| manager.join("app.slice")),
    };
    if let Some(pressure) = applications
        .map(|slice| slice.join("memory.pressure"))
        .filter(|path| path.exists())
    {
        sources.push(pressure);
    }
    sources
}

/// The user manager beside the logind session scope a process runs in (`0::/user.slice/
/// user-1000.slice/session-3.scope` gives `/sys/fs/cgroup/user.slice/user-1000.slice/
/// user@1000.service`), if it runs in one.
pub fn user_manager(cgroup: &str, uid: u32) -> Option<PathBuf> {
    let path = cgroup.lines().find_map(|line| line.strip_prefix("0::"))?;
    let scope = Path::new(path);
    let name = scope.file_name()?.to_str()?;
    if !(name.starts_with("session-") && name.ends_with(".scope")) {
        return None;
    }
    let slice = scope.parent()?.strip_prefix("/").ok()?;
    Some(
        Path::new("/sys/fs/cgroup")
            .join(slice)
            .join(format!("user@{uid}.service")),
    )
}

/// A pressure file's ten-second averages, in percent: some tasks waiting for memory, and all of
/// them (`some avg10=… avg60=… avg300=… total=…`, then `full …`).
pub fn averages(text: &str) -> Option<(f64, f64)> {
    let avg10 = |kind: &str| {
        text.lines()
            .find(|line| line.starts_with(kind))?
            .split_whitespace()
            .find_map(|field| field.strip_prefix("avg10=")?.parse::<f64>().ok())
    };
    Some((avg10("some")?, avg10("full")?))
}

/// The level of a file's averages.
pub fn level_of(some: f64, full: f64) -> MemoryPressureLevel {
    match oxyos_activity::pressure_level(some, full) {
        oxyos_activity::Level::Normal => MemoryPressureLevel::Normal,
        oxyos_activity::Level::Warning => MemoryPressureLevel::Warning,
        oxyos_activity::Level::Critical => MemoryPressureLevel::Critical,
    }
}

/// The session's follower of memory pressure, its thread waiting on the triggers.
pub struct MemoryPressure {
    level: MemoryPressureLevel,
    levels: mpsc::Receiver<MemoryPressureLevel>,
    woken: UnixStream,
    stop: Arc<Stop>,
    thread: Option<JoinHandle<()>>,
    followed: Vec<PathBuf>,
}

/// A trigger armed on a file: the file, its level, and whether it still works (a control group
/// removed hangs it up).
struct Armed {
    file: File,
    source: usize,
    level: MemoryPressureLevel,
}

impl MemoryPressure {
    /// Follows `sources`: each trigger armed now, a file the kernel refuses one on (no pressure
    /// stall information, a kernel too old for an unprivileged trigger) said once and left out.
    pub fn new(sources: Vec<PathBuf>) -> io::Result<Self> {
        let mut armed = Vec::new();
        let mut followed = Vec::new();
        for path in sources {
            let mut triggers = Vec::new();
            for (trigger, level) in TRIGGERS {
                match arm(&path, trigger) {
                    Ok(file) => triggers.push((file, level)),
                    Err(error) => {
                        eprintln!(
                            "[session] memory pressure: no trigger {trigger:?} on {}: {error}",
                            path.display()
                        );
                        triggers.clear();
                        break;
                    }
                }
            }
            if triggers.is_empty() {
                continue;
            }
            let source = followed.len();
            followed.push(path);
            armed.extend(triggers.into_iter().map(|(file, level)| Armed {
                file,
                source,
                level,
            }));
        }
        let (wake, woken) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        woken.set_nonblocking(true)?;
        let (levels_in, levels) = mpsc::channel();
        let stop = Arc::new(Stop::new()?);
        let thread = if armed.is_empty() {
            None
        } else {
            let (stop, files) = (stop.clone(), followed.clone());
            Some(
                std::thread::Builder::new()
                    .name("memory-pressure".into())
                    .spawn(move || follow(armed, &files, &stop, &levels_in, &wake))?,
            )
        };
        Ok(Self {
            level: MemoryPressureLevel::Normal,
            levels,
            woken,
            stop,
            thread,
            followed,
        })
    }

    /// The files followed, those the kernel took triggers on.
    pub fn followed(&self) -> &[PathBuf] {
        &self.followed
    }

    /// Readable when the level changed.
    pub fn wake_fd(&self) -> io::Result<OwnedFd> {
        Ok(self.woken.try_clone()?.into())
    }

    /// The level now, as the thread last said it.
    pub fn level(&self) -> MemoryPressureLevel {
        self.level
    }

    /// The level the thread said since, given the scene; and the clients told what it asks of
    /// them now (an application hidden since is told then).
    pub fn serve(&mut self, scene: &mut Scene) {
        let mut buffer = [0u8; 64];
        while matches!((&self.woken).read(&mut buffer), Ok(read) if read > 0) {}
        let mut changed = false;
        while let Ok(level) = self.levels.try_recv() {
            changed |= level != self.level;
            self.level = level;
        }
        if changed {
            eprintln!("[session] memory pressure: {:?}", self.level);
            scene.set_memory_pressure(self.level);
        }
        scene.tell_memory_pressure();
    }
}

impl Drop for MemoryPressure {
    fn drop(&mut self) {
        self.stop.give();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A trigger written to a pressure file (with its terminating NUL, as the kernel's
/// documentation writes one), the file kept open: closing it removes the trigger.
fn arm(path: &Path, trigger: &str) -> io::Result<File> {
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?;
    let mut line = trigger.as_bytes().to_vec();
    line.push(0);
    file.write_all(&line)?;
    Ok(file)
}

/// The thread: waits on every trigger (and the order to stop), forever while the level is
/// normal and 2 s at a time while it is up; says each change of level and wakes the session.
fn follow(
    mut armed: Vec<Armed>,
    files: &[PathBuf],
    stop: &Stop,
    levels: &mpsc::Sender<MemoryPressureLevel>,
    wake: &UnixStream,
) {
    let mut level = MemoryPressureLevel::Normal;
    let Ok(easing) = Timespec::try_from(EASING) else {
        return;
    };
    while !armed.is_empty() {
        let mut fds: Vec<PollFd<'_>> = Vec::with_capacity(armed.len() + 1);
        fds.push(PollFd::from_borrowed_fd(stop.fd(), PollFlags::IN));
        fds.extend(
            armed
                .iter()
                .map(|trigger| PollFd::from_borrowed_fd(trigger.file.as_fd(), PollFlags::PRI)),
        );
        let timeout = (level != MemoryPressureLevel::Normal).then_some(&easing);
        let ready = match poll(&mut fds, timeout) {
            Ok(ready) => ready,
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => {
                eprintln!("[session] memory pressure: {error}; no longer followed");
                return;
            }
        };
        if stop.given() {
            return;
        }
        let mut fired = MemoryPressureLevel::Normal;
        let mut gone = Vec::new();
        for (index, fd) in fds.iter().skip(1).enumerate() {
            let events = fd.revents();
            if events.intersects(PollFlags::ERR | PollFlags::HUP | PollFlags::NVAL) {
                gone.push(index);
            } else if events.contains(PollFlags::PRI) {
                fired = fired.max(armed[index].level);
            }
        }
        drop(fds);
        for index in gone.into_iter().rev() {
            let trigger = armed.remove(index);
            eprintln!(
                "[session] memory pressure: {} went; no longer followed",
                files[trigger.source].display()
            );
        }
        let next = if fired > level {
            fired
        } else if ready == 0 {
            // A while with no trigger: down as far as the averages say.
            files
                .iter()
                .filter_map(|path| std::fs::read_to_string(path).ok())
                .filter_map(|text| averages(&text))
                .map(|(some, full)| level_of(some, full))
                .max()
                .unwrap_or(MemoryPressureLevel::Normal)
                .max(fired)
        } else {
            level
        };
        if next != level {
            level = next;
            if levels.send(level).is_err() {
                return;
            }
            let _ = (&*wake).write(&[1]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pressure_files_averages_and_their_level() {
        let text = "some avg10=12.50 avg60=3.00 avg300=1.00 total=123456\n\
                    full avg10=2.25 avg60=0.50 avg300=0.10 total=6543\n";
        assert_eq!(averages(text), Some((12.5, 2.25)));
        assert_eq!(level_of(12.5, 2.25), MemoryPressureLevel::Warning);
        assert_eq!(level_of(41.0, 0.0), MemoryPressureLevel::Critical);
        assert_eq!(level_of(20.0, 10.0), MemoryPressureLevel::Critical);
        assert_eq!(level_of(9.9, 0.0), MemoryPressureLevel::Normal);
        assert_eq!(averages("some avg10=1.00\n"), None);
    }

    #[test]
    fn the_user_manager_beside_the_session_scope() {
        assert_eq!(
            user_manager("0::/user.slice/user-1002.slice/session-3.scope\n", 1002),
            Some(PathBuf::from(
                "/sys/fs/cgroup/user.slice/user-1002.slice/user@1002.service"
            ))
        );
        // A process the user manager started, or a service's, is in no session scope.
        assert_eq!(
            user_manager(
                "0::/user.slice/user-1000.slice/user@1000.service/app.slice/x.scope\n",
                1000
            ),
            None
        );
        assert_eq!(user_manager("0::/system.slice/ssh.service\n", 0), None);
    }

    #[test]
    fn a_file_without_triggers_is_left_out_and_nothing_waits() {
        let missing = std::env::temp_dir().join(format!("oxyos-no-pressure-{}", std::process::id()));
        let followed = MemoryPressure::new(vec![missing]).unwrap();
        assert!(followed.followed().is_empty());
        assert_eq!(followed.level(), MemoryPressureLevel::Normal);
    }

    /// On this machine's pressure, where the kernel takes an unprivileged trigger (Linux 6.5 and
    /// newer, its pressure stall information on): armed, waiting, and stopped at once.
    #[test]
    fn the_machines_triggers_are_armed_and_stopped() {
        if !Path::new(MACHINE).exists() {
            eprintln!("no {MACHINE} here");
            return;
        }
        let followed = MemoryPressure::new(vec![PathBuf::from(MACHINE)]).unwrap();
        if followed.followed().is_empty() {
            eprintln!("the kernel takes no unprivileged trigger here");
            return;
        }
        let started = std::time::Instant::now();
        drop(followed);
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
