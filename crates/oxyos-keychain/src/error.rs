//! What can go wrong, said without what was being handled: an error names the kind of thing and
//! never carries a key, a password, a plaintext, a ciphertext or a name.

use std::fmt;
use std::io;
use std::path::PathBuf;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Something received or read is not of its format; the text says what, never its content.
    Malformed(&'static str),
    /// A ciphertext did not open: the wrong key, or it (or what it is bound to) was changed.
    Decrypt,
    /// A signature does not verify, or its signer is not one the verified roster allows.
    Signature(&'static str),
    /// The roster does not verify.
    Roster(crate::roster::RosterError),
    /// An item older than one already seen (its counter went back), or Oxy's revision going back.
    Rollback,
    /// Oxy answered something other than what was asked or verified (another member, another
    /// enrollment, a ciphertext not the one sent): refused, never used.
    Answer(&'static str),
    /// A recovery key that is not the vault's.
    NotThisRecoveryKey,
    /// The keybag did not open with the password given.
    WrongPassword,
    /// The store already holds a keybag, or records sealed under one (made once; changed only by
    /// re-sealing it, or erased first).
    Exists,
    /// A file or directory of the store that anyone but the service could reach, or that is not
    /// what it should be (a link, another owner).
    Unsafe(PathBuf),
    /// The system's random generator failed.
    Random,
    /// A key could not be locked in memory (`mlock`, the process's `RLIMIT_MEMLOCK`).
    Memory(io::Error),
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Malformed(what) => write!(f, "malformed {what}"),
            Error::Decrypt => f.write_str("the ciphertext does not open"),
            Error::Signature(what) => write!(f, "signature refused: {what}"),
            Error::Roster(error) => write!(f, "roster refused: {error}"),
            Error::Rollback => f.write_str("an item older than one already seen"),
            Error::Answer(what) => write!(f, "Oxy's answer refused: {what}"),
            Error::NotThisRecoveryKey => f.write_str("not this vault's recovery key"),
            Error::WrongPassword => f.write_str("the keybag does not open with this password"),
            Error::Exists => f.write_str("the keychain's store is not empty"),
            Error::Unsafe(path) => write!(
                f,
                "{}: must be the service's own, not a link, and reachable by no one else",
                path.display()
            ),
            Error::Random => f.write_str("the system's random generator failed"),
            Error::Memory(error) => write!(f, "a key cannot be locked in memory: {error}"),
            Error::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(error) | Error::Memory(error) => Some(error),
            Error::Roster(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Error::Io(error)
    }
}

impl From<crate::roster::RosterError> for Error {
    fn from(error: crate::roster::RosterError) -> Self {
        Error::Roster(error)
    }
}
