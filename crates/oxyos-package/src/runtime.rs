//! A runtime's description, its `runtime.json` (docs/adr/0030-installing-applications.mdx,
//! decisions 2 and 17): the one reading of it, which the session's launcher (`oxyos-apps`) and the
//! install service (`oxyos-install`) both use.
//!
//! ```json
//! {"abi":1,"api":1,"framework":{"sha256":"…","size":13849134,"manifest":"…"}}
//! ```
//!
//! `abi` (from 1) and `api` (from 0), and the runtime's framework named by its bytecode's SHA-256
//! and size and its manifest's SHA-256 (`framework/framework.hbc`, `framework/framework.json`).
//! It is written by the runtime's build, not signed by anyone (it is the image's, in `/usr`), so it
//! need not be canonical; it is read strictly all the same: no key twice, no key unknown.

use crate::json::{self, Value};
use crate::{Error, Runtime, Sha256};

/// The most a `runtime.json` may be, in bytes.
pub const MAX_RUNTIME_DESCRIPTION: usize = 4096;

/// The runtime's framework, as its description names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameworkDigests {
    /// `framework/framework.hbc`'s.
    pub bytecode: Sha256,
    pub size: u64,
    /// `framework/framework.json`'s.
    pub manifest: Sha256,
}

/// A runtime's description.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeDescription {
    pub runtime: Runtime,
    pub framework: Option<FrameworkDigests>,
}

fn members(value: Value, object: &'static str) -> Result<Vec<(String, Value)>, Error> {
    match value {
        Value::Object(members) => Ok(members),
        _ => Err(Error::WrongType(object)),
    }
}

/// Takes `field` from `members`; refuses a missing one.
fn take(
    members: &mut Vec<(String, Value)>,
    object: &'static str,
    field: &'static str,
) -> Result<Value, Error> {
    let at = members
        .iter()
        .position(|(key, _)| key == field)
        .ok_or(Error::MissingField { object, field })?;
    Ok(members.remove(at).1)
}

fn integer(value: Value, field: &'static str) -> Result<u64, Error> {
    match value {
        Value::Integer(n) => Ok(n),
        _ => Err(Error::WrongType(field)),
    }
}

fn digest(value: Value, field: &'static str) -> Result<Sha256, Error> {
    match value {
        Value::String(text) => Sha256::parse(&text).ok_or(Error::Invalid(field)),
        _ => Err(Error::WrongType(field)),
    }
}

fn finish(members: Vec<(String, Value)>, object: &'static str) -> Result<(), Error> {
    match members.into_iter().next() {
        Some((field, _)) => Err(Error::UnknownField { object, field }),
        None => Ok(()),
    }
}

impl RuntimeDescription {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_RUNTIME_DESCRIPTION {
            return Err(Error::TooLarge("runtime.json"));
        }
        let mut top = members(json::parse(bytes)?, "runtime.json")?;
        let abi = integer(take(&mut top, "runtime.json", "abi")?, "abi")?;
        let api = integer(take(&mut top, "runtime.json", "api")?, "api")?;
        let framework = match top.iter().position(|(key, _)| key == "framework") {
            None => None,
            Some(at) => {
                let mut framework = members(top.remove(at).1, "framework")?;
                let digests = FrameworkDigests {
                    bytecode: digest(
                        take(&mut framework, "framework", "sha256")?,
                        "framework.sha256",
                    )?,
                    size: integer(take(&mut framework, "framework", "size")?, "framework.size")?,
                    manifest: digest(
                        take(&mut framework, "framework", "manifest")?,
                        "framework.manifest",
                    )?,
                };
                finish(framework, "framework")?;
                Some(digests)
            }
        };
        finish(top, "runtime.json")?;
        let abi = u32::try_from(abi)
            .ok()
            .filter(|&abi| abi > 0)
            .ok_or(Error::OutOfRange("abi"))?;
        let api = u32::try_from(api).map_err(|_| Error::OutOfRange("api"))?;
        Ok(Self {
            runtime: Runtime { abi, api },
            framework,
        })
    }
}
