//! `oxyos-pin-check`: an account's PIN, for PAM (docs/adr/0020-oxy-account-login.mdx, decision 2).
//!
//! PAM's `pam_exec` runs it with `expose_authtok`, under the PIN services (`oxyos-login-pin`, the
//! login program's, as root; `oxyos-pin`, the lock's, as the session's own user): the PIN on its
//! standard input (ended by a NUL), the account's user in `PAM_USER`. It asks `oxyos-accountd`,
//! which checks it against the account's PIN verifier, never online, counting a wrong one, and
//! answers only a peer's own account's (root's any). It exits 0 only when the PIN was accepted;
//! anything else (a wrong PIN, a wait, a locked-out PIN, no PIN, the service unreachable, an input
//! that is no PIN) is 1: PAM refuses, and nothing signs in.
//!
//! `oxyos-pin-check [--socket PATH]`: the service's socket, `/run/oxyos/accountd/socket` by
//! default (the tests' own otherwise). It prints nothing of the PIN, ever; a refusal's reason goes
//! to standard error, which `pam_exec` leaves out unless told otherwise (`quiet`).

use oxyos_account::ipc::{self, PinResult};
use oxyos_account::pin::MAX_PIN_DIGITS;
use oxyos_account::Secret;
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

fn run() -> Result<PinResult, String> {
    let mut socket = PathBuf::from(ipc::SOCKET);
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--socket" => {
                socket = arguments.next().ok_or("--socket needs a value")?.into();
            }
            _ => return Err(format!("unknown argument {argument}")),
        }
    }
    let user = std::env::var("PAM_USER").map_err(|_| "no PAM_USER")?;
    // At most a PIN and its NUL, plus one more byte to tell a longer input apart.
    let mut input = zeroize::Zeroizing::new(Vec::with_capacity(MAX_PIN_DIGITS + 2));
    std::io::stdin()
        .take((MAX_PIN_DIGITS + 2) as u64)
        .read_to_end(&mut input)
        .map_err(|error| format!("the PIN cannot be read: {error}"))?;
    let end = input.iter().position(|&b| b == 0).unwrap_or(input.len());
    let pin = std::str::from_utf8(&input[..end]).map_err(|_| "not a PIN")?;
    let answer = ipc::verify_pin_at(&socket, &user, Secret::new(pin))
        .map_err(|error| format!("oxyos-accountd: {error}"))?;
    Ok(answer.result)
}

fn main() -> ExitCode {
    match run() {
        Ok(PinResult::Accepted) => ExitCode::SUCCESS,
        Ok(result) => {
            eprintln!("oxyos-pin-check: {result:?}");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("oxyos-pin-check: {error}");
            ExitCode::FAILURE
        }
    }
}
