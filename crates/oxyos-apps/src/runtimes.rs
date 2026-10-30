//! The runtimes a machine has (docs/adr/0030-installing-applications.mdx, decision 2): Bloom host
//! builds side by side, one directory each, named by its `abi`:
//!
//! ```text
//! /usr/lib/oxyos/runtimes/<abi>/
//!     runtime.json        {"abi":1,"api":1}
//!     oxyos-bloom-host    the host, finding its libraries in lib/ ($ORIGIN/lib)
//!     lib/                React Native's and Hermes' libraries
//!     hermesc             the compiler of the Hermes the host runs (decision 1)
//! ```
//!
//! `abi` changes when an application must be rebuilt (a new React Native, a host module whose
//! JavaScript contract breaks); `api` grows when a runtime adds without breaking. A package runs on
//! a runtime of its `abi` whose `api` is at least its own. A Bloom application's entry names the
//! `abi` it was built for (`X-OxyOS-Runtime`); the session starts it with that runtime's host and
//! refuses one the machine lacks. Runtimes are the image's, in `/usr` ([`RUNTIMES`]), never an
//! environment variable's: the development compositor alone is given another directory, for
//! scenarios.

use super::Error;
use oxyos_package::{Runtime, RuntimeDescription, MAX_RUNTIME_DESCRIPTION};
use std::path::{Path, PathBuf};

/// Where the image keeps its runtimes.
pub const RUNTIMES: &str = "/usr/lib/oxyos/runtimes";

/// A runtime this machine has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledRuntime {
    /// Its numbers, as its `runtime.json` declares them (`oxyos_package::RuntimeDescription`,
    /// the one reading of it, the install service's too).
    pub level: Runtime,
    /// Its directory.
    pub directory: PathBuf,
    /// Its Bloom host.
    pub host: PathBuf,
    /// Its Hermes compiler.
    pub hermesc: PathBuf,
}

/// The runtime of `abi` in `runtimes`: its directory named by its number, its `runtime.json`
/// declaring that number, its host and compiler there. A runtime the machine lacks, or one whose
/// description says another `abi`, is an error saying so.
pub fn runtime(runtimes: &Path, abi: u32) -> Result<InstalledRuntime, Error> {
    let directory = runtimes.join(abi.to_string());
    let lacks = |why: String| {
        Error(format!(
            "runtime {abi} is not on this machine ({}: {why})",
            directory.display()
        ))
    };
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(directory.join("runtime.json"))
        .and_then(|file| {
            file.take(MAX_RUNTIME_DESCRIPTION as u64 + 1)
                .read_to_end(&mut bytes)
        })
        .map_err(|error| lacks(error.to_string()))?;
    let level = RuntimeDescription::parse(&bytes)
        .map_err(|error| lacks(format!("runtime.json: {error}")))?
        .runtime;
    if level.abi != abi {
        return Err(lacks(format!("its runtime.json says abi {}", level.abi)));
    }
    let file = |name: &str| {
        let path = directory.join(name);
        if path.is_file() {
            Ok(path)
        } else {
            Err(lacks(format!("no {name}")))
        }
    };
    Ok(InstalledRuntime {
        level,
        host: file("oxyos-bloom-host")?,
        hermesc: file("hermesc")?,
        directory,
    })
}

/// Every runtime in `runtimes`, by `abi`; directories that are not a runtime are said in the
/// errors.
pub fn installed(runtimes: &Path) -> (Vec<InstalledRuntime>, Vec<Error>) {
    let mut found = Vec::new();
    let mut errors = Vec::new();
    let Ok(entries) = std::fs::read_dir(runtimes) else {
        return (found, errors);
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(abi) = name
            .to_str()
            .filter(|name| !name.starts_with('0') && name.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|name| name.parse::<u32>().ok())
        else {
            errors.push(Error(format!(
                "{}: not a runtime's directory",
                entry.path().display()
            )));
            continue;
        };
        match runtime(runtimes, abi) {
            Ok(runtime) => found.push(runtime),
            Err(error) => errors.push(error),
        }
    }
    found.sort_by_key(|runtime| (runtime.level.abi, runtime.level.api));
    (found, errors)
}
