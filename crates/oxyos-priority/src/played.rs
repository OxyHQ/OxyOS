//! rtkit played on a bus of a run's own (`oxyos-offscreen --play-rtkit`), for the development
//! compositor's runs (the saturation benchmark, docs/performance.mdx): what a session on a seat is
//! given by the system's rtkit, which refuses a process in no seat session, given to the run's
//! compositor so it is measured as a session runs.
//!
//! It is rtkit's `org.freedesktop.RealtimeKit1` for high priority as rtkit 0.13 answers it: the
//! caller's own thread (its pid and user the bus's), no nice below `MinNiceLevel` (−15), the
//! thread marked `SCHED_RESET_ON_FORK` and given the nice; real time it refuses (the session never
//! asks it). And one rule of its own: only a thread in the run's control group (`run`) is ever
//! changed, so the capability it is given (`CAP_SYS_NICE`, by the run's wrapper and to this process
//! alone, never to the compositor or the applications) can raise nothing of the machine's own.
//! Every answer is said on its standard error and kept ([`Played::record`]).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// rtkit's defaults (`rtkit-daemon --min-nice-level=-15 --max-realtime-priority=20
/// --rttime-usec-max=200000`).
pub const MIN_NICE_LEVEL: i32 = -15;
pub const MAX_REALTIME_PRIORITY: i32 = 20;
pub const RTTIME_USEC_MAX: i64 = 200_000;

/// The played rtkit, served while this lives.
pub struct Played {
    _connection: zbus::blocking::Connection,
    record: Arc<Mutex<Vec<String>>>,
}

impl Played {
    /// Serves rtkit on the bus at `address` for the threads of the control group `run` (a path
    /// under `/sys/fs/cgroup`, or as `/proc/PID/cgroup` names it) and those below it.
    pub fn serve(address: &str, run: &Path) -> zbus::Result<Self> {
        let record = Arc::new(Mutex::new(Vec::new()));
        let connection = zbus::blocking::connection::Builder::address(address)?
            .name(super::RTKIT)?
            .serve_at(
                super::RTKIT_PATH,
                RealtimeKit {
                    run: hierarchy_path(run),
                    record: record.clone(),
                },
            )?
            .build()?;
        Ok(Self {
            _connection: connection,
            record,
        })
    }

    /// What it answered, oldest first: `granted PID/TID nice N` or `refused PID/TID …: why`.
    pub fn record(&self) -> Vec<String> {
        self.record.lock().unwrap().clone()
    }
}

/// A control group's path in the hierarchy (`/system.slice/x.service`), from a path under
/// `/sys/fs/cgroup` or one already in the hierarchy.
fn hierarchy_path(path: &Path) -> PathBuf {
    let path = path.strip_prefix("/sys/fs/cgroup").unwrap_or(path);
    Path::new("/").join(path)
}

/// Whether `inside` is `run` or below it.
fn within(inside: &Path, run: &Path) -> bool {
    inside.starts_with(run)
}

/// The control group of the thread `tid` of `pid` (cgroup v2: `0::/path`).
fn cgroup_of(pid: u64, tid: u64) -> std::io::Result<PathBuf> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/task/{tid}/cgroup"))?;
    text.lines()
        .find_map(|line| line.strip_prefix("0::"))
        .map(PathBuf::from)
        .ok_or_else(|| std::io::Error::other("no cgroup v2 line"))
}

struct RealtimeKit {
    run: PathBuf,
    record: Arc<Mutex<Vec<String>>>,
}

impl RealtimeKit {
    fn said(&self, line: String) {
        eprintln!("[played-rtkit] {line}");
        self.record.lock().unwrap().push(line);
    }

    /// rtkit's checks, then the thread given `priority`.
    fn high_priority(
        &self,
        caller: (u32, u32),
        process: u64,
        thread: u64,
        priority: i32,
    ) -> zbus::fdo::Result<()> {
        let refuse = |why: String| {
            self.said(format!("refused {process}/{thread} nice {priority}: {why}"));
            Err(zbus::fdo::Error::AccessDenied(why))
        };
        let (pid, uid) = caller;
        if process != u64::from(pid) {
            return refuse(format!("not the caller's own process ({pid})"));
        }
        let task = PathBuf::from(format!("/proc/{process}/task/{thread}"));
        let owner = match std::fs::metadata(&task) {
            Ok(metadata) => std::os::unix::fs::MetadataExt::uid(&metadata),
            Err(_) => return refuse("no such thread of that process".into()),
        };
        if owner != uid {
            return refuse(format!("another user's thread ({owner}, not {uid})"));
        }
        if priority < MIN_NICE_LEVEL {
            return refuse(format!("below the least nice, {MIN_NICE_LEVEL}"));
        }
        match cgroup_of(process, thread) {
            Ok(group) if within(&group, &self.run) => {}
            Ok(group) => {
                return refuse(format!(
                    "outside the run ({} is not in {})",
                    group.display(),
                    self.run.display()
                ))
            }
            Err(error) => return refuse(format!("its control group: {error}")),
        }
        if let Err(error) = give(thread, priority) {
            self.said(format!(
                "failed {process}/{thread} nice {priority}: {error}"
            ));
            return Err(zbus::fdo::Error::Failed(error.to_string()));
        }
        self.said(format!("granted {process}/{thread} nice {priority}"));
        Ok(())
    }
}

/// The thread `thread` at `SCHED_OTHER` with `SCHED_RESET_ON_FORK`, then nice `priority`, as rtkit's
/// `thread_set_high_priority` does.
fn give(thread: u64, priority: i32) -> std::io::Result<()> {
    let tid = libc::pid_t::try_from(thread).map_err(std::io::Error::other)?;
    let parameters = libc::sched_param { sched_priority: 0 };
    #[allow(unsafe_code)]
    // SAFETY: `parameters` is a live, initialised `sched_param` for the call's duration; the call
    // changes the policy of the thread `tid` alone.
    let scheduled = unsafe {
        libc::sched_setscheduler(
            tid,
            libc::SCHED_OTHER | libc::SCHED_RESET_ON_FORK,
            &parameters,
        )
    };
    if scheduled != 0 {
        return Err(std::io::Error::last_os_error());
    }
    #[allow(unsafe_code)]
    // SAFETY: setpriority takes integers alone; `PRIO_PROCESS` with a thread's id changes that
    // thread's nice alone.
    let niced = unsafe { libc::setpriority(libc::PRIO_PROCESS, tid as libc::id_t, priority) };
    if niced != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// The caller's pid and user, as the bus knows them.
async fn caller(
    connection: &zbus::Connection,
    header: &zbus::message::Header<'_>,
) -> zbus::fdo::Result<(u32, u32)> {
    let sender = header
        .sender()
        .ok_or_else(|| zbus::fdo::Error::AccessDenied("no sender".into()))?;
    let bus = zbus::fdo::DBusProxy::new(connection).await?;
    let name = zbus::names::BusName::Unique(sender.to_owned());
    let pid = bus.get_connection_unix_process_id(name.clone()).await?;
    let uid = bus.get_connection_unix_user(name).await?;
    Ok((pid, uid))
}

#[zbus::interface(name = "org.freedesktop.RealtimeKit1")]
impl RealtimeKit {
    async fn make_thread_high_priority(
        &self,
        thread: u64,
        priority: i32,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        let caller = caller(connection, &header).await?;
        self.high_priority(caller, u64::from(caller.0), thread, priority)
    }

    #[zbus(name = "MakeThreadHighPriorityWithPID")]
    async fn make_thread_high_priority_with_pid(
        &self,
        process: u64,
        thread: u64,
        priority: i32,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        let caller = caller(connection, &header).await?;
        self.high_priority(caller, process, thread, priority)
    }

    fn make_thread_realtime(&self, thread: u64, priority: u32) -> zbus::fdo::Result<()> {
        self.said(format!("refused real time {thread} {priority}: not played"));
        Err(zbus::fdo::Error::NotSupported(
            "real time is not played".into(),
        ))
    }

    #[zbus(name = "MakeThreadRealtimeWithPID")]
    fn make_thread_realtime_with_pid(
        &self,
        process: u64,
        thread: u64,
        priority: u32,
    ) -> zbus::fdo::Result<()> {
        self.said(format!(
            "refused real time {process}/{thread} {priority}: not played"
        ));
        Err(zbus::fdo::Error::NotSupported(
            "real time is not played".into(),
        ))
    }

    #[zbus(property, name = "MinNiceLevel")]
    fn min_nice_level(&self) -> i32 {
        MIN_NICE_LEVEL
    }

    #[zbus(property, name = "MaxRealtimePriority")]
    fn max_realtime_priority(&self) -> i32 {
        MAX_REALTIME_PRIORITY
    }

    #[zbus(property, name = "RTTimeUSecMax")]
    fn rttime_usec_max(&self) -> i64 {
        RTTIME_USEC_MAX
    }
}
