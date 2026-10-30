//! `oxyos-accountd`, the system service that holds the device credential of every Oxy account
//! set up on the machine ([ADR 0020](../../../docs/adr/0020-oxy-account-login.mdx), decision 3).
//! It runs as root, because it makes users and sets their verifiers, and nothing else runs with
//! it; `oxyos-accountd.service` confines it to what that needs.
//!
//! This is its first step: setting an account up (`oxyos-account`'s `setup`), for the setup client
//! on its socket. Only root and the login program's user may enroll.
//!
//! `oxyos-accountd [--state DIRECTORY] [--socket PATH] [--api URL] [--enroller USER]`:
//! - `--state`: the enrolled accounts, `/var/lib/oxyos/accounts` by default;
//! - `--socket`: where it listens, `/run/oxyos/accountd/socket` by default;
//! - `--api`: Oxy's API, `https://api.oxy.so` by default (plain HTTP only to this machine, for a
//!   development server);
//! - `--enroller`: the login program's user, `oxyos-login` by default, read from `/etc/passwd` at
//!   each connection; while it has no entry, only root may enroll.
//!
//! It logs what each setup came to (the account's user, the reason it was refused) on standard
//! error, the journal's; never a password, a secret, a code or a token.

use oxyos_account::api::HttpApi;
use oxyos_account::ipc::{self, Enrollers, Listener, Outcome, Peer};
use oxyos_account::setup::Setup;
use oxyos_account::store::{Store, STATE_DIRECTORY};
use oxyos_account::users::{passwd_entry, SystemUsers};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Conversations at once; more wait to be accepted.
const MAX_CONVERSATIONS: usize = 4;

struct Options {
    state: PathBuf,
    socket: PathBuf,
    api: String,
    enroller: String,
}

fn options() -> Result<Options, String> {
    let mut options = Options {
        state: STATE_DIRECTORY.into(),
        socket: ipc::SOCKET.into(),
        api: oxyos_account::api::OXY_API.into(),
        enroller: "oxyos-login".into(),
    };
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        let mut value = || {
            arguments
                .next()
                .ok_or_else(|| format!("{argument} needs a value"))
        };
        match argument.as_str() {
            "--state" => options.state = value()?.into(),
            "--socket" => options.socket = value()?.into(),
            "--api" => options.api = value()?,
            "--enroller" => options.enroller = value()?,
            _ => {
                return Err(format!(
                    "unknown argument {argument}\n\
                     usage: oxyos-accountd [--state DIRECTORY] [--socket PATH] [--api URL] \
                     [--enroller USER]"
                ))
            }
        }
    }
    Ok(options)
}

fn log(peer: &Peer, outcome: &Outcome) {
    let who = format!("uid {} (pid {})", peer.uid, peer.pid);
    match outcome {
        Outcome::Enrolled(enrolled) => eprintln!(
            "oxyos-accountd: {who} set up {} as {} (uid {})",
            enrolled.account, enrolled.user, enrolled.uid
        ),
        Outcome::Refused(error) => eprintln!("oxyos-accountd: {who}: setup refused: {error}"),
        Outcome::SecondFactorRequired => {
            eprintln!("oxyos-accountd: {who}: setup waits for the authenticator's code")
        }
        Outcome::InvalidRequest => eprintln!("oxyos-accountd: {who}: not a request"),
    }
}

/// The login program's user, if it has one.
fn enrollers(name: &str) -> Enrollers {
    let login = match passwd_entry(Path::new("/etc/passwd"), name) {
        Ok(entry) => entry.map(|user| user.uid),
        Err(error) => {
            eprintln!("oxyos-accountd: {error}: only root may enroll");
            None
        }
    };
    Enrollers { login }
}

fn run() -> Result<(), String> {
    let options = options()?;
    if !rustix::process::geteuid().is_root() {
        return Err("it runs as root: it makes users and sets their verifiers".into());
    }
    let api = HttpApi::new(&options.api)?;
    let store = Store::open(&options.state)
        .map_err(|error| format!("{}: {error}", options.state.display()))?;
    let setup = Arc::new(Setup::new(api, SystemUsers::new(), store));
    let listener = Listener::bind(&options.socket)
        .map_err(|error| format!("{}: {error}", options.socket.display()))?;
    eprintln!(
        "oxyos-accountd: listening on {}, accounts in {}, Oxy at {}",
        options.socket.display(),
        options.state.display(),
        options.api
    );
    let open = Arc::new(AtomicUsize::new(0));
    loop {
        let connection = match listener.accept() {
            Ok(connection) => connection,
            Err(error) => {
                eprintln!("oxyos-accountd: accepting a connection: {error}");
                continue;
            }
        };
        if open.fetch_add(1, Ordering::SeqCst) >= MAX_CONVERSATIONS {
            open.fetch_sub(1, Ordering::SeqCst);
            let _ = connection.send(&ipc::Answer::refused(ipc::Reason::Busy));
            continue;
        }
        let setup = setup.clone();
        let open_now = open.clone();
        let enrollers = enrollers(&options.enroller);
        let spawned = std::thread::Builder::new()
            .name("oxyos-accountd-conversation".into())
            .spawn(move || {
                if let Err(error) = ipc::serve(&connection, &setup, enrollers, log) {
                    eprintln!("oxyos-accountd: a conversation ended: {error}");
                }
                open_now.fetch_sub(1, Ordering::SeqCst);
            });
        if let Err(error) = spawned {
            open.fetch_sub(1, Ordering::SeqCst);
            eprintln!("oxyos-accountd: a conversation cannot start: {error}");
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("oxyos-accountd: {error}");
            ExitCode::FAILURE
        }
    }
}
