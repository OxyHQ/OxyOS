//! Processes known by pidfd, never by a pid alone (a pid is reused once its process is reaped):
//! a socket peer's (`SO_PEERPIDFD`, Linux 6.5), a launched process's (`pidfd_open` by the session
//! right after it started it, handed over with `SCM_RIGHTS`).
//!
//! What a pidfd's process is, is read from `/proc` by its pid, and the pidfd is read again after:
//! a pidfd whose process has been reaped says so (its pid reads as -1), so a pid that still reads
//! the same was that process's throughout.

use rustix::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// `SO_PEERPIDFD` (`asm-generic/socket.h`; the same on every architecture OxyOS runs on).
const SO_PEERPIDFD: libc::c_int = 77;

/// The socket's peer, as a pidfd of the process that connected.
pub fn peer_pidfd(socket: BorrowedFd<'_>) -> io::Result<OwnedFd> {
    let mut fd: libc::c_int = -1;
    let mut length = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    #[allow(unsafe_code)]
    // SAFETY: `socket` is an open descriptor for the call's duration; the kernel writes at most
    // `length` bytes (one `int`) to `fd`, a live local of that size, and the length to `length`.
    let answered = unsafe {
        libc::getsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            SO_PEERPIDFD,
            (&mut fd as *mut libc::c_int).cast(),
            &mut length,
        )
    };
    if answered != 0 {
        return Err(io::Error::last_os_error());
    }
    if fd < 0 || length as usize != std::mem::size_of::<libc::c_int>() {
        return Err(io::Error::other("SO_PEERPIDFD gave no descriptor"));
    }
    #[allow(unsafe_code)]
    // SAFETY: the kernel just installed `fd` in this process's table for this call (close-on-exec,
    // as every pidfd is); nothing else owns it.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// A pidfd of the process `pid` (a child the caller has not reaped, so the pid is still its).
pub fn open(pid: u32) -> io::Result<OwnedFd> {
    let pid = i32::try_from(pid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a pid"))?;
    Ok(rustix::process::pidfd_open(
        pid,
        rustix::process::PidfdFlags::empty(),
    )?)
}

/// The pid of a pidfd's process while it has one; `None` once it has been reaped. Fails if `fd`
/// is not a pidfd.
pub fn pid(fd: BorrowedFd<'_>) -> io::Result<Option<i32>> {
    let info = std::fs::read_to_string(format!("/proc/self/fdinfo/{}", fd.as_raw_fd()))?;
    let pid = info
        .lines()
        .find_map(|line| line.strip_prefix("Pid:"))
        .and_then(|value| value.trim().parse::<i32>().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a pidfd"))?;
    Ok((pid > 0).then_some(pid))
}

/// Whether a pidfd's process has exited (it is readable then).
pub fn exited(fd: BorrowedFd<'_>) -> bool {
    use rustix::event::{poll, PollFd, PollFlags};
    let mut fds = [PollFd::new(&fd, PollFlags::IN)];
    let zero = rustix::time::Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    poll(&mut fds, Some(&zero)).is_ok_and(|ready| ready > 0)
}

/// What `/proc/<pid>/status` says of a process: its parent and its real user id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub parent: i32,
    pub uid: u32,
}

pub fn status(pid: i32) -> io::Result<Status> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/status"))?;
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|value| value.split_whitespace().next())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("no {name}")))
    };
    let parse = |value: &str| {
        value
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "not a number"))
    };
    Ok(Status {
        parent: parse(field("PPid:")?)?,
        uid: parse(field("Uid:")?)? as u32,
    })
}

/// What a program file is: its device and inode (what runs, not the path it was found by), its
/// owner and its mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Program {
    pub device: u64,
    pub inode: u64,
    pub owner: u32,
    pub mode: u32,
    pub file: bool,
}

impl Program {
    fn of(metadata: &std::fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            owner: metadata.uid(),
            mode: metadata.mode(),
            file: metadata.is_file(),
        }
    }

    /// The program at `path`.
    pub fn at(path: &Path) -> io::Result<Self> {
        Ok(Self::of(&std::fs::metadata(path)?))
    }

    /// The program `pid` runs: the file its `/proc` entry's `exe` link resolves to, whatever path
    /// (or mount namespace) it was started from.
    pub fn of_process(pid: i32) -> io::Result<Self> {
        Ok(Self::of(&std::fs::metadata(format!("/proc/{pid}/exe"))?))
    }

    /// The same file as `other`, a regular file owned by `owner` that neither its group nor
    /// others may write.
    pub fn is(&self, other: &Program, owner: u32) -> bool {
        self.device == other.device
            && self.inode == other.inode
            && self.file
            && self.owner == owner
            && self.mode & 0o022 == 0
    }
}

/// The pid of `fd`'s process, read before and after `look` ran on it: `look`'s answer if the
/// process was the same throughout, else `None`.
pub fn while_alive<T>(fd: impl AsFd, look: impl FnOnce(i32) -> io::Result<T>) -> Option<(i32, T)> {
    let before = pid(fd.as_fd()).ok()??;
    let answer = look(before).ok()?;
    (pid(fd.as_fd()).ok()? == Some(before)).then_some((before, answer))
}
