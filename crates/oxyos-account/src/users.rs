//! The Unix side of an enrolled account: its user, its home and its verifier, made with the
//! system's own tools, as an administrator would make them (passwd's `useradd`, `chpasswd`,
//! `userdel`). Each runs from an argument vector, never a shell, with a clean environment.
//!
//! - **The user:** `useradd --create-home --user-group`: an ordinary user (uid in login.defs'
//!   `UID_MIN`–`UID_MAX`), its own group, its home from the skeleton at login.defs' `HOME_MODE`
//!   (0700 on Debian). It is made with its password locked (`!`), so until its verifier is set
//!   nothing can sign in as it.
//! - **The verifier:** `chpasswd`, the password on its standard input (never an argument, which
//!   any process may read in `/proc`). It hashes as the system is configured to: through PAM's
//!   `common-password` on Debian (`pam_unix … yescrypt`), else login.defs' `ENCRYPT_METHOD`
//!   (`YESCRYPT` on Debian trixie). It takes the user database's lock and replaces the shadow file
//!   whole. `pam_chauthtok` from this process would do the same through the same modules, with a
//!   conversation of its own to answer their prompts and a PAM binding in a root service; the tool
//!   is the system's own path to the same result, and is what an administrator's scripts use.
//! - **Removal:** `userdel --remove`: the user, its group and its home.
//!
//! Users are read from `/etc/passwd` (passwd(5)), where `useradd` writes them. A name another
//! account database holds is refused by `useradd` itself ("already exists").
//!
//! [`SystemUsers::with_prefix`] runs the same tools on another root's files (`--prefix`), which
//! they do without privileges: the tests make real users, verifiers and homes that way.

use crate::id::UserName;
use crate::secret::Secret;
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use zeroize::Zeroizing;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnixUser {
    pub uid: u32,
    pub gid: u32,
    pub home: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UsersError {
    /// A user of that name already exists.
    Exists,
    /// No user of that name exists.
    Missing,
    /// The tool failed, or the account database could not be read.
    Failed(String),
}

impl fmt::Display for UsersError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UsersError::Exists => f.write_str("a user of that name already exists"),
            UsersError::Missing => f.write_str("no user of that name exists"),
            UsersError::Failed(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for UsersError {}

pub trait Users {
    fn lookup(&self, name: &UserName) -> Result<Option<UnixUser>, UsersError>;
    /// A new user with its own group and home, its password locked.
    fn create(&self, name: &UserName) -> Result<UnixUser, UsersError>;
    /// The user's password, as the system hashes it.
    fn set_verifier(&self, name: &UserName, password: &Secret) -> Result<(), UsersError>;
    /// The user, its group and its home.
    fn remove(&self, name: &UserName) -> Result<(), UsersError>;
}

/// The longest password the verifier takes: Linux-PAM's longest answer (`PAM_MAX_RESP_SIZE`),
/// which is also the most the lock screen sends ([ADR 0010](../../docs/adr/0010-lock-screen.mdx)).
pub const MAX_PASSWORD_BYTES: usize = 512;

/// Whether `password` can be a verifier: not empty, at most [`MAX_PASSWORD_BYTES`], and without a
/// NUL (no C string holds one) or a line break (`chpasswd` reads a line, and no password field
/// can type one).
pub fn verifier_password(password: &Secret) -> bool {
    let bytes = password.expose().as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_PASSWORD_BYTES
        && !bytes.iter().any(|b| matches!(b, 0 | b'\n' | b'\r'))
}

pub struct SystemUsers {
    prefix: Option<PathBuf>,
    pub(crate) useradd: PathBuf,
    pub(crate) chpasswd: PathBuf,
    pub(crate) userdel: PathBuf,
}

impl Default for SystemUsers {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemUsers {
    /// The system's own users.
    pub fn new() -> Self {
        Self {
            prefix: None,
            useradd: "/usr/sbin/useradd".into(),
            chpasswd: "/usr/sbin/chpasswd".into(),
            userdel: "/usr/sbin/userdel".into(),
        }
    }

    /// The users of the system whose root is `prefix` (`prefix/etc/passwd`, `prefix/home`).
    pub fn with_prefix(prefix: &Path) -> Self {
        Self {
            prefix: Some(prefix.to_owned()),
            ..Self::new()
        }
    }

    fn passwd(&self) -> PathBuf {
        self.prefix
            .as_deref()
            .unwrap_or(Path::new("/"))
            .join("etc/passwd")
    }

    fn command(&self, tool: &Path) -> Command {
        let mut command = Command::new(tool);
        command
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if let Some(prefix) = &self.prefix {
            command.arg("--prefix").arg(prefix);
        }
        command
    }
}

/// What a tool's failure was, by its exit status and what it said.
fn failed(tool: &str, output: &std::process::Output) -> UsersError {
    let said = String::from_utf8_lossy(&output.stderr);
    UsersError::Failed(format!(
        "{tool} failed ({}): {}",
        output.status,
        said.trim()
    ))
}

fn run(tool: &str, command: &mut Command) -> Result<std::process::Output, UsersError> {
    command
        .output()
        .map_err(|error| UsersError::Failed(format!("{tool} cannot run: {error}")))
}

/// `name`'s entry in a passwd(5) file.
pub fn passwd_entry(passwd: &Path, name: &str) -> Result<Option<UnixUser>, UsersError> {
    let text = std::fs::read_to_string(passwd)
        .map_err(|error| UsersError::Failed(format!("{}: {error}", passwd.display())))?;
    for line in text.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.first() != Some(&name) {
            continue;
        }
        let number = |index: usize| fields.get(index).and_then(|field| field.parse().ok());
        return match (fields.len(), number(2), number(3)) {
            (7, Some(uid), Some(gid)) => Ok(Some(UnixUser {
                uid,
                gid,
                home: PathBuf::from(fields[5]),
            })),
            _ => Err(UsersError::Failed(format!(
                "{}: {name}'s entry is malformed",
                passwd.display()
            ))),
        };
    }
    Ok(None)
}

impl Users for SystemUsers {
    fn lookup(&self, name: &UserName) -> Result<Option<UnixUser>, UsersError> {
        passwd_entry(&self.passwd(), name.as_str())
    }

    fn create(&self, name: &UserName) -> Result<UnixUser, UsersError> {
        let output = run(
            "useradd",
            self.command(&self.useradd)
                .args(["--create-home", "--user-group", "--"])
                .arg(name.as_str()),
        )?;
        match output.status.code() {
            Some(0) => {}
            // useradd(8): 9, the user name is already in use.
            Some(9) => return Err(UsersError::Exists),
            _ => return Err(failed("useradd", &output)),
        }
        self.lookup(name)?
            .ok_or_else(|| UsersError::Failed(format!("useradd made no user {name}")))
    }

    fn set_verifier(&self, name: &UserName, password: &Secret) -> Result<(), UsersError> {
        if !verifier_password(password) {
            return Err(UsersError::Failed(
                "the password cannot be a verifier".into(),
            ));
        }
        let mut line = Zeroizing::new(String::with_capacity(
            name.as_str().len() + password.expose().len() + 2,
        ));
        line.push_str(name.as_str());
        line.push(':');
        line.push_str(password.expose());
        line.push('\n');
        let mut child = self
            .command(&self.chpasswd)
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|error| UsersError::Failed(format!("chpasswd cannot run: {error}")))?;
        let written = child
            .stdin
            .take()
            .map(|mut stdin| stdin.write_all(line.as_bytes()))
            .unwrap_or_else(|| Err(std::io::ErrorKind::BrokenPipe.into()));
        drop(line);
        let output = child
            .wait_with_output()
            .map_err(|error| UsersError::Failed(format!("chpasswd: {error}")))?;
        // What chpasswd says is not repeated: the input it read was a password.
        if !output.status.success() || written.is_err() {
            return Err(UsersError::Failed(format!(
                "chpasswd failed ({})",
                output.status
            )));
        }
        Ok(())
    }

    fn remove(&self, name: &UserName) -> Result<(), UsersError> {
        let output = run(
            "userdel",
            self.command(&self.userdel)
                .args(["--remove", "--"])
                .arg(name.as_str()),
        )?;
        match output.status.code() {
            Some(0) => Ok(()),
            // userdel(8): 6, the user does not exist.
            Some(6) => Err(UsersError::Missing),
            _ => Err(failed("userdel", &output)),
        }
    }
}
