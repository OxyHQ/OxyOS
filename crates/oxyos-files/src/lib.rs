//! What an OxyOS application reaches of the file system (docs/adr/0027-files.mdx): the user's home
//! and the removable media mounted for them, and nothing else.
//!
//! - [`Scope`] decides what a path names: an absolute path with no `.` or `..`, inside the home or
//!   a mounted volume once its links are resolved, and never inside a Trash, which only the
//!   Trash's own operations reach. Everything else is refused, with the reason.
//! - [`entries`] lists a folder and creates, renames and moves items, never replacing one.
//! - [`trash`] moves items to the Trash as the freedesktop.org Trash specification (1.0) says,
//!   and lists, restores and removes what it holds. Nothing here deletes an item that is not in
//!   the Trash.
//! - [`places`]: the home, its favourite folders (the XDG user directories) and the volumes.
//! - [`watch`]: a folder's, the Trash's and the volumes' changes, through inotify and the mount
//!   table.
//! - [`Service`] runs requests in order on a thread of its own and answers them, and the changes
//!   watched, through one descriptor the Bloom host polls (`OxyOSFiles`).

use std::fmt;
use std::io;

pub mod entries;
pub mod places;
mod scope;
pub mod service;
pub mod trash;
pub mod watch;

pub use scope::Scope;
pub use service::Service;

/// Why a request was not done.
#[derive(Debug)]
pub enum Error {
    /// The path is outside what an application may reach, or names what it may not change (the
    /// home itself, a volume's root, a Trash's insides): the reason.
    Refused(String),
    /// Nothing is there.
    NotFound,
    /// Something is already there with that name.
    Exists,
    /// A folder was wanted.
    NotAFolder,
    /// A name that cannot be an item's: empty, with a `/`, `.` or `..`, or longer than 255 bytes.
    InvalidName,
    /// The item and where it goes are on different volumes: a move would be a copy.
    CrossDevice,
    /// The system did not allow it.
    Denied,
    /// Anything else the system answered.
    Io(io::Error),
}

impl Error {
    /// The error's name in answers, which the application turns into its own words.
    pub fn code(&self) -> &'static str {
        match self {
            Error::Refused(_) => "refused",
            Error::NotFound => "not-found",
            Error::Exists => "exists",
            Error::NotAFolder => "not-a-folder",
            Error::InvalidName => "invalid-name",
            Error::CrossDevice => "cross-device",
            Error::Denied => "denied",
            Error::Io(_) => "io",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Refused(reason) => write!(f, "refused: {reason}"),
            Error::NotFound => f.write_str("nothing is there"),
            Error::Exists => f.write_str("an item with that name is already there"),
            Error::NotAFolder => f.write_str("not a folder"),
            Error::InvalidName => f.write_str("not a name an item can have"),
            Error::CrossDevice => f.write_str("on another volume"),
            Error::Denied => f.write_str("not allowed by the system"),
            Error::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        if error.raw_os_error() == Some(rustix::io::Errno::XDEV.raw_os_error()) {
            return Error::CrossDevice;
        }
        if error.raw_os_error() == Some(rustix::io::Errno::NOTDIR.raw_os_error()) {
            return Error::NotAFolder;
        }
        match error.kind() {
            io::ErrorKind::NotFound => Error::NotFound,
            io::ErrorKind::AlreadyExists => Error::Exists,
            io::ErrorKind::PermissionDenied => Error::Denied,
            _ => Error::Io(error),
        }
    }
}

impl From<rustix::io::Errno> for Error {
    fn from(errno: rustix::io::Errno) -> Self {
        Error::from(io::Error::from(errno))
    }
}

/// Whether `name` can be an item's name: not empty or only spaces, no `/` or NUL, not `.` or
/// `..`, at most 255 bytes (the longest name Linux's file systems keep).
pub fn valid_name(name: &str) -> Result<(), Error> {
    if name.trim().is_empty()
        || name.len() > 255
        || name.contains(['/', '\0'])
        || name == "."
        || name == ".."
    {
        return Err(Error::InvalidName);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
