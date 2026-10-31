//! What an OxyOS application reaches of the file system (docs/adr/0028-file-access.mdx): the user's
//! home, the removable media mounted for them and the application's own folders, and nothing
//! else; beneath `expo-file-system`'s API and the Bloom adapter's places and Trash.
//!
//! - [`Scope`] decides what a path names: an absolute path with no `.` or `..`, inside the home, a
//!   mounted volume or the application's folders once its links are resolved (its bundle read,
//!   never changed), and never inside a Trash, which only the Trash's own operations reach.
//!   Everything else is refused, with the reason.
//! - [`access`]: `expo-file-system`'s operations as its Android implementation does them: facts,
//!   reading, writing, creating, deleting, copying and moving (never over an item unless asked),
//!   listing, digests and open files, by `file://` URI ([`uri`]).
//! - [`entries`] lists a folder and renames items, never replacing one.
//! - [`trash`] moves items to the Trash as the freedesktop.org Trash specification (1.0) says,
//!   and lists, restores and removes what it holds.
//! - [`places`]: the home, its favourite folders (the XDG user directories) and the volumes.
//! - [`watch`]: what changes in a folder or a file, typed and debounced as `expo-file-system`'s
//!   watchers say it, and whether the Trash or the volumes changed, through inotify and the mount
//!   table.
//! - [`transfer`]: downloads into a file and uploads of one, through `oxyos-net`.
//! - [`Service`] answers requests on the caller's thread (`expo-file-system`'s synchronous calls)
//!   or in order on a thread of its own, and says the changes watched, through one descriptor the
//!   Bloom host polls (`OxyOSFiles`).

use std::fmt;
use std::io;

pub mod access;
pub mod entries;
pub mod mime;
mod mounts;
pub mod places;
mod scope;
pub mod service;
pub mod transfer;
pub mod trash;
pub mod uri;
pub mod watch;

pub use scope::{Access, Application, Root, Scope};
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
    /// The folder a copy, a move or a download would go into does not exist.
    NoDestination,
    /// A folder was wanted, and something else is there.
    NotAFolder,
    /// A file was wanted, and a folder is there.
    NotAFile,
    /// A name that cannot be an item's: empty, with a `/`, `.` or `..`, or longer than 255 bytes.
    InvalidName,
    /// The item and where it goes are on different volumes: a move would be a copy.
    CrossDevice,
    /// The system did not allow it.
    Denied,
    /// A request's argument that cannot be: its reason (a mode, an encoding, an algorithm).
    Invalid(String),
    /// What OxyOS cannot do yet: its reason.
    Unsupported(String),
    /// The application cancelled it.
    Cancelled,
    /// A transfer the network could not make, or whose server refused it: the reason.
    Network(String),
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
            Error::NoDestination => "no-destination",
            Error::NotAFolder => "not-a-folder",
            Error::NotAFile => "not-a-file",
            Error::InvalidName => "invalid-name",
            Error::CrossDevice => "cross-device",
            Error::Denied => "denied",
            Error::Invalid(_) => "invalid",
            Error::Unsupported(_) => "unsupported",
            Error::Cancelled => "cancelled",
            Error::Network(_) => "network",
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
            Error::NoDestination => f.write_str("the destination's folder does not exist"),
            Error::NotAFolder => f.write_str("not a folder"),
            Error::NotAFile => f.write_str("a folder is there, not a file"),
            Error::InvalidName => f.write_str("not a name an item can have"),
            Error::CrossDevice => f.write_str("on another volume"),
            Error::Denied => f.write_str("not allowed by the system"),
            Error::Invalid(reason) | Error::Unsupported(reason) | Error::Network(reason) => {
                f.write_str(reason)
            }
            Error::Cancelled => f.write_str("cancelled"),
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
mod access_tests;
#[cfg(test)]
mod mounts_tests;
#[cfg(test)]
mod tests;
