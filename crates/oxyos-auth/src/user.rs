//! A user of the system's account database, and starting a program as that user, as a login does
//! (`login(1)`, a login manager's session): its groups (`initgroups`' list, read before the
//! child is made), its group and user ids, its home, a new session of processes, and an
//! environment of the caller's choosing alone.

use std::ffi::{CString, OsStr};
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
    /// The user named `name`, if the account database has one (through NSS, `oxyos-users`).
    pub fn named(name: &str) -> io::Result<Option<Self>> {
        let Some(user) = oxyos_users::user_named(name)? else {
            return Ok(None);
        };
        let Ok(c_name) = CString::new(name) else {
            return Ok(None);
        };
        let groups = oxyos_users::group_list(&c_name, user.gid)?;
        Ok(Some(Self {
            name: name.to_owned(),
            uid: user.uid,
            gid: user.gid,
            home: user.home,
            shell: user.shell,
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
