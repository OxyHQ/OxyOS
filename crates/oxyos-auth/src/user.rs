//! A user of the system's account database, and starting a program as that user, as a login does
//! (`login(1)`, a login manager's session): its groups (`initgroups`' list, read before the
//! child is made), its group and user ids, its home, a new session of processes, and an
//! environment of the caller's choosing alone.

use std::ffi::{CStr, CString, OsStr};
use std::io;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

/// A user, from the account database.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub home: PathBuf,
    pub shell: PathBuf,
    /// Every group it belongs to, its own first.
    pub groups: Vec<u32>,
}

impl User {
    /// The user named `name`, if the account database has one.
    #[allow(unsafe_code)]
    pub fn named(name: &str) -> io::Result<Option<Self>> {
        let Ok(c_name) = CString::new(name) else {
            return Ok(None);
        };
        let mut buffer = vec![0 as libc::c_char; 16 * 1024];
        // SAFETY: an all-zero passwd is a valid value of the C struct (null pointers, zero ids).
        let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
        let mut found: *mut libc::passwd = std::ptr::null_mut();
        // SAFETY: every pointer is valid and `buffer` is as long as said; getpwnam_r fills
        // `entry` with strings inside `buffer` and sets `found` to `entry` or leaves it null.
        let status = unsafe {
            libc::getpwnam_r(
                c_name.as_ptr(),
                &mut entry,
                buffer.as_mut_ptr(),
                buffer.len(),
                &mut found,
            )
        };
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status));
        }
        if found.is_null() {
            return Ok(None);
        }
        let string = |pointer: *const libc::c_char| {
            if pointer.is_null() {
                String::new()
            } else {
                // SAFETY: the entry's strings point into `buffer`, NUL-terminated, while it lives.
                unsafe { CStr::from_ptr(pointer) }
                    .to_string_lossy()
                    .into_owned()
            }
        };
        let groups = group_list(&c_name, entry.pw_gid)?;
        Ok(Some(Self {
            name: name.to_owned(),
            uid: entry.pw_uid,
            gid: entry.pw_gid,
            home: string(entry.pw_dir).into(),
            shell: string(entry.pw_shell).into(),
            groups,
        }))
    }

    /// Starts `program` with `args` as this user, from its home, in a session of its own, with
    /// exactly `env` as its environment, its input from `/dev/null` and its output the
    /// caller's (the journal's, for a service). Only root can.
    #[allow(unsafe_code)]
    pub fn spawn(
        &self,
        program: &OsStr,
        args: &[impl AsRef<OsStr>],
        env: &[(String, String)],
    ) -> io::Result<Child> {
        let (uid, gid) = (self.uid, self.gid);
        let groups: Vec<libc::gid_t> = self.groups.clone();
        let mut command = Command::new(program);
        command
            .args(args)
            .env_clear()
            .envs(env.iter().map(|(name, value)| (name, value)))
            .current_dir(&self.home)
            .stdin(Stdio::null());
        // SAFETY: the closure runs in the child between fork and exec, the child having one
        // thread; it makes only system calls (setsid, setgroups, setgid, setuid, getuid,
        // getgid), which are async-signal-safe, and allocates nothing: the groups were read
        // before the fork.
        unsafe {
            command.pre_exec(move || {
                if libc::setsid() < 0
                    || libc::setgroups(groups.len(), groups.as_ptr()) != 0
                    || libc::setgid(gid) != 0
                    || libc::setuid(uid) != 0
                {
                    return Err(io::Error::last_os_error());
                }
                // Never run as anyone else: the ids must be the user's and stay so.
                if libc::getuid() != uid || libc::geteuid() != uid || libc::getgid() != gid {
                    return Err(io::Error::from_raw_os_error(libc::EPERM));
                }
                if uid != 0 && libc::setuid(0) == 0 {
                    return Err(io::Error::from_raw_os_error(libc::EPERM));
                }
                Ok(())
            });
        }
        command.spawn()
    }
}

/// The groups `name` belongs to (getgrouplist), `gid` first.
#[allow(unsafe_code)]
fn group_list(name: &CStr, gid: u32) -> io::Result<Vec<u32>> {
    let mut count: libc::c_int = 32;
    loop {
        let mut groups = vec![0 as libc::gid_t; count as usize];
        let asked = count;
        // SAFETY: `groups` holds `count` ids; getgrouplist writes at most that many and sets
        // `count` to how many there are.
        let found =
            unsafe { libc::getgrouplist(name.as_ptr(), gid, groups.as_mut_ptr(), &mut count) };
        if found >= 0 {
            groups.truncate(count as usize);
            return Ok(groups);
        }
        if count <= asked {
            // It found more than fitted but says no more: a broken database.
            count = asked.saturating_mul(2);
        }
        if count > 65536 {
            return Err(io::Error::other("too many groups"));
        }
    }
}
