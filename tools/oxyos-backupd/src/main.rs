//! `oxyos-backupd`, the system service that backs the computer up, as Time Machine does
//! ([ADR 0059](../../../docs/adr/0059-backups.mdx)).
//!
//! `oxyos-backupd [--state DIRECTORY] [--socket PATH] [--volumes DIRECTORY] [--cache DIRECTORY]`:
//! the service, as root (`oxyos-backupd.service`): it reads every home, mounts backup disks in its
//! own mount namespace and keeps the destinations' keys, root's alone.
//! - `--state`: its directory, `/var/lib/oxyos/backup` by default;
//! - `--socket`: where it listens, `/run/oxyos/backupd/socket` by default;
//! - `--volumes`: where it mounts disks, `/run/oxyos/backupd/volumes` by default;
//! - `--cache`: restic's cache, `/var/cache/oxyos/backup` by default.
//!
//! `oxyos-backupd --development --state DIRECTORY --volumes DIRECTORY --source PATH… [--socket
//! PATH] [--cache DIRECTORY]`: a developer's service, for the development compositor's scenarios
//! (as `oxyos-installd --development`): unprivileged, never root, serving its own user alone; it
//! backs up the `--source` paths, and each folder of `--volumes` is a disk connected, its name the
//! file system's UUID. restic and rclone are those on `PATH`.
//!
//! `oxyos-backupd --migrate [--socket PATH] [--target ROOT] [--volumes DIRECTORY] [--cache
//! DIRECTORY]`: *Migrate from a Backup*, the login screen's, as root (`oxyos-migrate.service`,
//! started by `oxyos-migrate.socket`, whose listening socket is its standard input): it restores
//! a computer's backup into the running system while no account is enrolled
//! (`crates/oxyos-backup/src/migrate.rs`), and ends once nobody has connected for a minute.
//! - `--socket`: listens there itself, rather than on the socket systemd passed;
//! - `--target`: the root restored into, `/` by default;
//! - `--volumes`: where it mounts disks, `/run/oxyos/migrate-volumes` by default.
//!
//! `oxyos-backupd --client [--socket PATH] REQUEST`: sends one request (JSON, as the socket's
//! protocol writes it, `crates/oxyos-backup/src/ipc.rs`) and prints each answer on a line, until
//! the answer to it (a watch: until interrupted). For trying the service by hand.
//!
//! The service logs on standard error, the journal's.

use oxyos_backup::disks;
use oxyos_backup::engine::Engine;
use oxyos_backup::ipc;
use oxyos_backup::migrate::{self, Migration};
use oxyos_backup::restore::Restorer;
use oxyos_backupd::Arguments as Service;
use oxyos_install::socket::{Connection, Listener};
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

/// How long the migration service waits for another conversation before it ends.
const MIGRATE_IDLE: rustix::event::Timespec = rustix::event::Timespec {
    tv_sec: 60,
    tv_nsec: 0,
};

const USAGE: &str = "usage: oxyos-backupd [--state DIRECTORY] [--socket PATH] [--volumes DIRECTORY] [--cache DIRECTORY]\n       \
oxyos-backupd --development --state DIRECTORY --volumes DIRECTORY --source PATH... [--socket PATH] [--cache DIRECTORY]\n       \
oxyos-backupd --migrate [--socket PATH] [--target ROOT] [--volumes DIRECTORY] [--cache DIRECTORY]\n       \
oxyos-backupd --client [--socket PATH] REQUEST";

struct Arguments {
    client: Option<String>,
    /// The migration service (`--migrate`), restoring into `target`.
    migrate: bool,
    target: Option<PathBuf>,
    service: Service,
}

fn arguments() -> Result<Arguments, String> {
    let mut client = None;
    let (mut migrate, mut target) = (false, None);
    let mut parsed = Service::default();
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        let mut value = |name: &str| {
            arguments
                .next()
                .ok_or_else(|| format!("{name} needs a value\n{USAGE}"))
        };
        match argument.as_str() {
            "--client" => client = Some(value("--client")?),
            "--development" => parsed.development = true,
            "--migrate" => migrate = true,
            "--target" => target = Some(PathBuf::from(value("--target")?)),
            "--state" => parsed.state = value("--state")?.into(),
            "--socket" => parsed.socket = Some(value("--socket")?.into()),
            "--volumes" => parsed.volumes = Some(value("--volumes")?.into()),
            "--cache" => parsed.cache = Some(value("--cache")?.into()),
            "--source" => parsed.sources.push(value("--source")?.into()),
            "--help" | "-h" => return Err(USAGE.into()),
            other => return Err(format!("unknown argument {other}\n{USAGE}")),
        }
    }
    Ok(Arguments {
        client,
        migrate,
        target,
        service: parsed,
    })
}

fn client(arguments: &Arguments, request: &str) -> Result<(), String> {
    let request: serde_json::Value =
        serde_json::from_str(request).map_err(|e| format!("the request is not JSON: {e}"))?;
    let watching = request == "watch";
    let socket = arguments
        .service
        .socket
        .clone()
        .unwrap_or_else(|| ipc::SOCKET.into());
    let connection =
        Connection::connect(&socket).map_err(|e| format!("{}: {e}", socket.display()))?;
    connection.send(&request).map_err(|e| e.to_string())?;
    loop {
        match connection.receive::<serde_json::Value>() {
            Ok(Some(Some(answer))) => {
                println!("{answer}");
                if !watching {
                    return match answer.get("refused") {
                        Some(_) => Err("refused".into()),
                        None => Ok(()),
                    };
                }
            }
            Ok(Some(None)) => return Err("an answer that is not JSON".into()),
            Ok(None) => return Err("the service closed the connection".into()),
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// The migration service: one conversation at a time, until none comes for a minute.
fn migration(arguments: Arguments) -> Result<(), String> {
    if !rustix::process::geteuid().is_root() {
        return Err("--migrate runs as root: it makes users and writes their homes".into());
    }
    let service = &arguments.service;
    let cache = service
        .cache
        .clone()
        .unwrap_or_else(|| "/var/cache/oxyos/backup".into());
    std::fs::create_dir_all(&cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    let volumes = service
        .volumes
        .clone()
        .unwrap_or_else(|| "/run/oxyos/migrate-volumes".into());
    let disks = disks::System::open(&volumes).map_err(|e| format!("{}: {e}", volumes.display()))?;
    let target = arguments.target.clone().unwrap_or_else(|| "/".into());
    let migration = Migration {
        restorer: Restorer::new(Engine::new(&cache), Arc::new(disks)),
        reload_networks: target == Path::new("/"),
        target,
        login: migrate::login_user(Path::new("/etc/passwd")),
    };
    let listener = match &service.socket {
        Some(socket) => {
            Listener::bind(socket, 0o666).map_err(|e| format!("{}: {e}", socket.display()))?
        }
        // systemd's listening socket, on standard input (`StandardInput=socket`).
        None => {
            let stdin = std::io::stdin()
                .as_fd()
                .try_clone_to_owned()
                .map_err(|e| format!("standard input: {e}"))?;
            let is_socket = rustix::fs::fstat(&stdin)
                .is_ok_and(|stat| rustix::fs::FileType::from_raw_mode(stat.st_mode).is_socket());
            if !is_socket {
                return Err(format!(
                    "--migrate without --socket listens on the socket systemd passes\n{USAGE}"
                ));
            }
            Listener::from_fd(stdin)
        }
    };
    loop {
        let mut ready = [rustix::event::PollFd::new(
            &listener,
            rustix::event::PollFlags::IN,
        )];
        match rustix::event::poll(&mut ready, Some(&MIGRATE_IDLE)) {
            Ok(0) => return Ok(()),
            Ok(_) => {}
            Err(rustix::io::Errno::INTR) => continue,
            Err(e) => return Err(format!("waiting for a conversation: {e}")),
        }
        match listener.accept() {
            Ok(connection) => {
                if let Err(e) = migration.serve(&connection) {
                    eprintln!("oxyos-migrate: a conversation ended: {e}");
                }
            }
            Err(e) => eprintln!("oxyos-migrate: accepting a connection: {e}"),
        }
    }
}

fn main() -> ExitCode {
    let result = arguments().and_then(|arguments| match arguments.client.clone() {
        Some(request) => client(&arguments, &request),
        None if arguments.migrate => migration(arguments),
        None => oxyos_backupd::run(arguments.service),
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("oxyos-backupd: {e}");
            ExitCode::FAILURE
        }
    }
}
