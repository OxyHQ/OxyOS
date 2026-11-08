//! Quitting one of the person's processes (docs/adr/0063-activity-monitor.mdx, decision 6): the
//! process named by its pid and its start, signalled through a pidfd opened on it and checked to
//! be it, so a pid the kernel gave another since the person chose is never signalled. Only the
//! session's user's processes (its real, effective and saved user), never the session itself nor
//! the processes it runs under; another user's need an administrator, whom no session asks yet.

use crate::procfs::{self, Procfs};
use rustix::process::{pidfd_open, pidfd_send_signal, Pid, PidfdFlags, Signal};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// It is gone, or its pid is another process's now.
    NotFound,
    /// Another user's, or the session's own.
    NotPermitted,
    Failed,
}

impl Failure {
    pub fn name(self) -> &'static str {
        match self {
            Failure::NotFound => "notFound",
            Failure::NotPermitted => "notPermitted",
            Failure::Failed => "failed",
        }
    }
}

/// The session's process and those it runs under (its parent, theirs, up to the first): ending
/// one ends the session, which signing out does.
pub fn protected(procfs: &Procfs, session: u32) -> Vec<u32> {
    let mut buffer = String::new();
    let mut chain = vec![session];
    let mut pid = session;
    while let Some(stat) = procfs::stat(procfs, pid, &mut buffer) {
        if stat.ppid == 0 || chain.contains(&stat.ppid) {
            break;
        }
        chain.push(stat.ppid);
        pid = stat.ppid;
    }
    chain
}

/// Ends the process `pid` that started at `started` (SIGTERM, or SIGKILL when `force`), if it is
/// `uid`'s and not `protected`.
pub fn quit(
    procfs: &Procfs,
    pid: u32,
    started: u64,
    force: bool,
    uid: u32,
    protected: &[u32],
) -> Result<(), Failure> {
    if protected.contains(&pid) {
        return Err(Failure::NotPermitted);
    }
    let raw = i32::try_from(pid).map_err(|_| Failure::NotFound)?;
    let target = Pid::from_raw(raw).ok_or(Failure::NotFound)?;
    // Held from here: whatever the pid becomes, this descriptor is the process it named now.
    let pidfd = pidfd_open(target, PidfdFlags::empty()).map_err(|_| Failure::NotFound)?;
    let mut buffer = String::new();
    // The process the descriptor holds is the one the person chose only if it started then: read
    // after the descriptor was opened, a pid used again since shows another start.
    let stat = procfs::stat(procfs, pid, &mut buffer).ok_or(Failure::NotFound)?;
    if stat.started != started {
        return Err(Failure::NotFound);
    }
    let status = procfs::status(procfs, pid, &mut buffer).ok_or(Failure::NotFound)?;
    if !status.uids[..3].iter().all(|id| *id == uid) {
        return Err(Failure::NotPermitted);
    }
    let signal = if force { Signal::KILL } else { Signal::TERM };
    pidfd_send_signal(&pidfd, signal).map_err(|error| match error {
        rustix::io::Errno::SRCH => Failure::NotFound,
        rustix::io::Errno::PERM => Failure::NotPermitted,
        _ => Failure::Failed,
    })
}
