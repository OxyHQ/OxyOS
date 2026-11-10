//! `oxyos-backupd`'s service, run by its program (`tools/oxyos-backupd`) and by the development
//! compositor (`oxyos-offscreen --play-backup`), which plays it on a run's own folders for the
//! scenarios ([ADR 0059](../../docs/adr/0059-backups.mdx), step 2) as the program's
//! `--development` does. It lives here, not in the program, so that no program depends on another.

use crate::disks::{self, Disks};
use crate::engine::Engine;
use crate::ipc;
use crate::policy::{self, Sources};
use crate::service::{self, Options, Service};
use crate::store::STATE_DIRECTORY;
use oxyos_install::socket::Listener;
use oxyos_seat::notify::{wait_readable, Notify};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Connections at once: every session keeps one to follow the state, Settings one, Dropsea a few.
const MAX_CONNECTIONS: usize = 64;

/// What the service runs with.
pub struct Arguments {
    /// A developer's service: unprivileged, on folders (`--development`).
    pub development: bool,
    pub state: PathBuf,
    pub socket: Option<PathBuf>,
    pub volumes: Option<PathBuf>,
    pub cache: Option<PathBuf>,
    pub sources: Vec<PathBuf>,
}

impl Default for Arguments {
    fn default() -> Self {
        Self {
            development: false,
            state: STATE_DIRECTORY.into(),
            socket: None,
            volumes: None,
            cache: None,
            sources: Vec::new(),
        }
    }
}

fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .map(|h| h.trim().to_owned())
        .unwrap_or_default()
}

/// Runs the service: it listens on its socket and serves until the process ends.
pub fn run(arguments: Arguments, notify: Option<Notify>) -> Result<(), String> {
    let root = rustix::process::geteuid().is_root();
    if arguments.development {
        if root {
            return Err("--development is a developer's, never root's".into());
        }
        if arguments.state == std::path::Path::new(STATE_DIRECTORY) {
            return Err("--development needs a --state of its own".into());
        }
        if arguments.volumes.is_none() || arguments.sources.is_empty() {
            return Err("--development needs --volumes and --source".into());
        }
    } else if !root {
        return Err(
            "it runs as root: it reads every home and keeps the destinations' keys, root's alone"
                .into(),
        );
    }
    let cache = arguments
        .cache
        .clone()
        .unwrap_or_else(|| "/var/cache/oxyos/backup".into());
    std::fs::create_dir_all(&cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    let (engine, disks, sources): (Engine, Arc<dyn Disks>, Sources) = if arguments.development {
        let engine = Engine::from_path(&cache).ok_or("restic is not on PATH")?;
        let volumes = arguments.volumes.clone().unwrap_or_default();
        let disks =
            disks::Played::open(&volumes).map_err(|e| format!("{}: {e}", volumes.display()))?;
        let sources = Sources {
            paths: arguments.sources.clone(),
            excludes: Vec::new(),
        };
        (engine, Arc::new(disks), sources)
    } else {
        let volumes = arguments
            .volumes
            .clone()
            .unwrap_or_else(|| "/run/oxyos/backupd/volumes".into());
        let disks =
            disks::System::open(&volumes).map_err(|e| format!("{}: {e}", volumes.display()))?;
        (Engine::new(&cache), Arc::new(disks), Sources::machine())
    };
    let options = Options {
        state: arguments.state.clone(),
        engine,
        disks,
        sources,
        passwd: "/etc/passwd".into(),
        group: "/etc/group".into(),
        accounts: (!arguments.development).then(|| "/var/lib/oxyos/accounts".into()),
        manifest: (!arguments.development).then(|| policy::MANIFEST_DIRECTORY.into()),
        development: arguments.development,
        clock: Arc::new(service::now),
    };
    let service = Service::open(options, &hostname())
        .map_err(|e| format!("{}: {e}", arguments.state.display()))?;
    let socket = arguments
        .socket
        .clone()
        .unwrap_or_else(|| ipc::SOCKET.into());
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    // Anyone may connect: the peer's credentials decide what it may ask.
    let listener =
        Listener::bind(&socket, 0o666).map_err(|e| format!("{}: {e}", socket.display()))?;
    eprintln!(
        "oxyos-backupd: listening on {}, its state in {}",
        socket.display(),
        arguments.state.display()
    );
    let scheduler = service.clone();
    std::thread::Builder::new()
        .name("oxyos-backupd-schedule".into())
        .spawn(move || scheduler.schedule())
        .map_err(|e| format!("the schedule cannot start: {e}"))?;
    let open = Arc::new(AtomicUsize::new(0));
    loop {
        // The accept loop says it is alive while it waits (the unit's WatchdogSec=).
        if let Err(e) = wait_readable(&listener, notify.as_ref()) {
            eprintln!("oxyos-backupd: waiting for a connection: {e}");
        }
        let connection = match listener.accept() {
            Ok(connection) => connection,
            Err(e) => {
                eprintln!("oxyos-backupd: accepting a connection: {e}");
                continue;
            }
        };
        if open.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            open.fetch_sub(1, Ordering::SeqCst);
            eprintln!("oxyos-backupd: too many connections at once: one closed unanswered");
            continue;
        }
        let service = service.clone();
        let open_now = open.clone();
        let spawned = std::thread::Builder::new()
            .name("oxyos-backupd-conversation".into())
            .spawn(move || {
                if let Err(e) = service.serve(&connection) {
                    eprintln!("oxyos-backupd: a conversation ended: {e}");
                }
                open_now.fetch_sub(1, Ordering::SeqCst);
            });
        if let Err(e) = spawned {
            open.fetch_sub(1, Ordering::SeqCst);
            eprintln!("oxyos-backupd: a conversation cannot start: {e}");
        }
    }
}
