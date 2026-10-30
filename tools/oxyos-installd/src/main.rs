//! `oxyos-installd`, the system service that installs, updates and removes applications
//! ([ADR 0030](../../../docs/adr/0030-installing-applications.mdx), decision 5), and its fetcher.
//!
//! `oxyos-installd [--state DIRECTORY] [--anchor FILE] [--runtimes DIRECTORY] [--socket PATH]
//! [--fetcher PATH] [--passwd FILE]`: the service. It runs as root with no capabilities and no
//! network (`oxyos-installd.service`): it verifies the store's metadata and every file, compiles,
//! assembles, switches and removes versions in its own directory, and never runs a package's code
//! or touches a home.
//! - `--state`: its directory, `/var/lib/oxyos/apps` by default;
//! - `--anchor`: the image's trusted root of the store's metadata,
//!   `/usr/share/oxyos/store/root.json` by default;
//! - `--runtimes`: the image's runtimes, `/usr/lib/oxyos/runtimes` by default;
//! - `--socket`: where it listens, `/run/oxyos/installd/socket` by default;
//! - `--fetcher`: the fetcher's socket, `/run/oxyos/app-fetch/socket` by default;
//! - `--passwd`: the user database, `/etc/passwd` by default.
//!
//! `oxyos-installd --fetch [--repository URL] [--socket PATH]`: the fetcher, run as a dynamic user
//! with the network alone (`oxyos-app-fetch.service`). It writes the repository's files into the
//! descriptors the service hands it, for root alone. `--repository` is `file:///…` in this step.
//!
//! Both log on standard error, the journal's.

use oxyos_install::accounts::Passwd;
use oxyos_install::compile::Hermesc;
use oxyos_install::fetch::{self, FetcherClient, LocalRepository};
use oxyos_install::install::{self, Installer, Machine};
use oxyos_install::ipc::{self, Outcome};
use oxyos_install::socket::{Listener, Peer};
use oxyos_install::state::{Layout, STATE_DIRECTORY};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Connections at once: every session keeps one for its holds, Atlas one, the timer one.
const MAX_CONNECTIONS: usize = 32;

const USAGE: &str = "usage: oxyos-installd [--state DIRECTORY] [--anchor FILE] [--runtimes DIRECTORY] \
[--socket PATH] [--fetcher PATH] [--passwd FILE]\n       oxyos-installd --fetch [--repository URL] [--socket PATH]";

struct Options {
    fetcher_mode: bool,
    state: PathBuf,
    anchor: PathBuf,
    runtimes: PathBuf,
    socket: Option<PathBuf>,
    fetcher: PathBuf,
    passwd: PathBuf,
    repository: String,
}

fn options() -> Result<Options, String> {
    let mut options = Options {
        fetcher_mode: false,
        state: STATE_DIRECTORY.into(),
        anchor: install::ANCHOR.into(),
        runtimes: install::RUNTIMES.into(),
        socket: None,
        fetcher: fetch::FETCH_SOCKET.into(),
        passwd: "/etc/passwd".into(),
        repository: "file:///var/cache/oxyos/store".into(),
    };
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        let mut value = || {
            arguments
                .next()
                .ok_or_else(|| format!("{argument} needs a value"))
        };
        match argument.as_str() {
            "--fetch" => options.fetcher_mode = true,
            "--state" => options.state = value()?.into(),
            "--anchor" => options.anchor = value()?.into(),
            "--runtimes" => options.runtimes = value()?.into(),
            "--socket" => options.socket = Some(value()?.into()),
            "--fetcher" => options.fetcher = value()?.into(),
            "--passwd" => options.passwd = value()?.into(),
            "--repository" => options.repository = value()?,
            _ => return Err(format!("unknown argument {argument}\n{USAGE}")),
        }
    }
    Ok(options)
}

fn log(peer: &Peer, outcome: &Outcome) {
    let who = format!("uid {} (pid {})", peer.uid, peer.pid);
    let id = outcome.id.as_deref().unwrap_or("-");
    match &outcome.result {
        Ok(what) => eprintln!("oxyos-installd: {who}: {} {id}: {what}", outcome.request),
        Err(why) => eprintln!(
            "oxyos-installd: {who}: {} {id} refused: {why}",
            outcome.request
        ),
    }
}

fn fetcher(options: &Options) -> Result<(), String> {
    if rustix::process::geteuid().is_root() {
        return Err("the fetcher runs as a user of its own, never root".into());
    }
    let repository = LocalRepository::from_url(&options.repository)?;
    let socket = options
        .socket
        .clone()
        .unwrap_or_else(|| fetch::FETCH_SOCKET.into());
    // Any process may connect (the install service is root without CAP_DAC_OVERRIDE, and the
    // socket is the dynamic user's): the peer's credentials decide, and only root is served.
    let listener =
        Listener::bind(&socket, 0o666).map_err(|e| format!("{}: {e}", socket.display()))?;
    eprintln!(
        "oxyos-installd --fetch: listening on {}, the repository {}",
        socket.display(),
        options.repository
    );
    let open = Arc::new(AtomicUsize::new(0));
    loop {
        let connection = match listener.accept() {
            Ok(connection) => connection,
            Err(e) => {
                eprintln!("oxyos-installd --fetch: accepting: {e}");
                continue;
            }
        };
        if open.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            open.fetch_sub(1, Ordering::SeqCst);
            continue;
        }
        let repository = repository.clone();
        let open_now = open.clone();
        let spawned = std::thread::Builder::new()
            .name("oxyos-fetch".into())
            .spawn(move || {
                // Root is the install service; nobody else may have files written.
                if let Err(e) = fetch::serve(&connection, &repository, |peer| peer.uid == 0) {
                    eprintln!("oxyos-installd --fetch: a connection ended: {e}");
                }
                open_now.fetch_sub(1, Ordering::SeqCst);
            });
        if spawned.is_err() {
            open.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

fn service(options: &Options) -> Result<(), String> {
    if !rustix::process::geteuid().is_root() {
        return Err(
            "it runs as root: the applications' files are root's, so only an administrator \
             could have written them"
                .into(),
        );
    }
    let machine = Machine::discover(&options.runtimes, install::machine_architecture());
    let installer = Installer::open(
        Layout::new(&options.state),
        &options.anchor,
        FetcherClient::new(&options.fetcher),
        Hermesc::for_this_process(),
        Passwd::new(&options.passwd),
        machine.clone(),
        oxyos_install::time::now,
    )
    .map_err(|e| format!("{}: {e}", options.state.display()))?;
    let installer = Arc::new(installer);
    let socket = options.socket.clone().unwrap_or_else(|| ipc::SOCKET.into());
    let listener =
        Listener::bind(&socket, 0o666).map_err(|e| format!("{}: {e}", socket.display()))?;
    eprintln!(
        "oxyos-installd: listening on {}, applications in {}, runtimes {:?}",
        socket.display(),
        options.state.display(),
        machine
            .runtimes
            .iter()
            .map(|(r, _)| format!("{}.{}", r.abi, r.api))
            .collect::<Vec<_>>()
    );
    let open = Arc::new(AtomicUsize::new(0));
    loop {
        let connection = match listener.accept() {
            Ok(connection) => connection,
            Err(e) => {
                eprintln!("oxyos-installd: accepting a connection: {e}");
                continue;
            }
        };
        if open.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            open.fetch_sub(1, Ordering::SeqCst);
            eprintln!("oxyos-installd: too many connections at once: one closed unanswered");
            continue;
        }
        let installer = installer.clone();
        let open_now = open.clone();
        let spawned = std::thread::Builder::new()
            .name("oxyos-installd-conversation".into())
            .spawn(move || {
                if let Err(e) = ipc::serve(&connection, &installer, log) {
                    eprintln!("oxyos-installd: a conversation ended: {e}");
                }
                open_now.fetch_sub(1, Ordering::SeqCst);
            });
        if let Err(e) = spawned {
            open.fetch_sub(1, Ordering::SeqCst);
            eprintln!("oxyos-installd: a conversation cannot start: {e}");
        }
    }
}

fn main() -> ExitCode {
    let result = options().and_then(|options| {
        if options.fetcher_mode {
            fetcher(&options)
        } else {
            service(&options)
        }
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("oxyos-installd: {e}");
            ExitCode::FAILURE
        }
    }
}
