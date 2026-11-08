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
//! `oxyos-backupd --client [--socket PATH] REQUEST`: sends one request (JSON, as the socket's
//! protocol writes it, `crates/oxyos-backup/src/ipc.rs`) and prints each answer on a line, until
//! the answer to it (a watch: until interrupted). For trying the service by hand.
//!
//! The service logs on standard error, the journal's.

use oxyos_backup::ipc;
use oxyos_backupd::Arguments as Service;
use oxyos_install::socket::Connection;
use std::process::ExitCode;

const USAGE: &str = "usage: oxyos-backupd [--state DIRECTORY] [--socket PATH] [--volumes DIRECTORY] [--cache DIRECTORY]\n       \
oxyos-backupd --development --state DIRECTORY --volumes DIRECTORY --source PATH... [--socket PATH] [--cache DIRECTORY]\n       \
oxyos-backupd --client [--socket PATH] REQUEST";

struct Arguments {
    client: Option<String>,
    service: Service,
}

fn arguments() -> Result<Arguments, String> {
    let mut client = None;
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

fn main() -> ExitCode {
    let result = arguments().and_then(|arguments| match arguments.client.clone() {
        Some(request) => client(&arguments, &request),
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
