//! The session's connection to the install service (docs/adr/0030-installing-applications.mdx,
//! decisions 5 and 10): for each installed version it launches, a hold while it runs (the service
//! never removes a version a session runs), `shown` once it showed a window (the previous version
//! may go), and `exited` when it ends, saying whether it showed one (two exits without one roll the
//! version back).
//!
//! It is a thread of its own, told by the session's loop and never waited for: the compositor's
//! frames never depend on the service. Holds end with their connection, so the thread keeps one,
//! and if it is lost (the service restarted) it connects again and holds again what still runs.
//! Without the service (a development machine) notes are dropped, said once.

use oxyos_install::ipc::Request;
use oxyos_install::socket::Connection;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

/// What the session tells the service of an installed version it launched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    Hold {
        id: String,
        version_code: u64,
    },
    Shown {
        id: String,
        version_code: u64,
    },
    Exited {
        id: String,
        version_code: u64,
        shown: bool,
    },
}

/// How long the service may take to answer.
const ANSWER: Duration = Duration::from_secs(5);

pub struct Installd {
    notes: mpsc::Sender<Note>,
}

impl Installd {
    /// The connection to the service at `socket` (`oxyos_install::ipc::SOCKET`), on its thread.
    pub fn start(socket: PathBuf) -> Self {
        let (notes, received) = mpsc::channel();
        std::thread::Builder::new()
            .name("installd".into())
            .spawn(move || serve(socket, received))
            .expect("the install service's thread starts");
        Self { notes }
    }

    pub fn note(&self, note: Note) {
        let _ = self.notes.send(note);
    }
}

/// Each version held, as the service must be told again on a new connection: one hold for each
/// running process of it.
#[derive(Default)]
struct Holds(Vec<(String, u64)>);

impl Holds {
    fn versions(&self) -> BTreeSet<(String, u64)> {
        self.0.iter().cloned().collect()
    }
}

fn serve(socket: PathBuf, notes: mpsc::Receiver<Note>) {
    let mut holds = Holds::default();
    let mut connection: Option<Connection> = None;
    let mut said = false;
    for note in notes {
        // What still runs, after this note: a process exited lets one hold of its version go.
        let released = match &note {
            Note::Hold { id, version_code } => {
                holds.0.push((id.clone(), *version_code));
                None
            }
            Note::Exited {
                id, version_code, ..
            } => {
                let at = holds
                    .0
                    .iter()
                    .position(|(i, v)| i == id && v == version_code);
                at.map(|at| holds.0.remove(at))
                    .filter(|held| !holds.0.contains(held))
            }
            Note::Shown { .. } => None,
        };
        if connection.is_none() {
            match connect(&socket, &holds) {
                Ok(connected) => {
                    connection = Some(connected);
                    said = false;
                }
                Err(error) => {
                    if !said {
                        eprintln!(
                            "[session] the install service ({}): {error}; its versions are not held",
                            socket.display()
                        );
                        said = true;
                    }
                    continue;
                }
            }
            // A new connection was told every hold, this note's among them.
            if matches!(note, Note::Hold { .. }) {
                continue;
            }
        }
        let mut requests = vec![request(&note)];
        if let Some((id, version_code)) = released {
            requests.push(Request::Release {
                id,
                version_code,
                uid: None,
            });
        }
        for request in requests {
            if let Err(error) = ask(connection.as_ref().expect("connected"), &request) {
                eprintln!("[session] the install service: {error}");
                connection = None;
                break;
            }
        }
    }
}

fn request(note: &Note) -> Request {
    match note.clone() {
        Note::Hold { id, version_code } => Request::Hold {
            id,
            version_code,
            uid: None,
        },
        Note::Shown { id, version_code } => Request::Shown {
            id,
            version_code,
            uid: None,
        },
        Note::Exited {
            id,
            version_code,
            shown,
        } => Request::Exited {
            id,
            version_code,
            shown,
            uid: None,
        },
    }
}

/// A connection to the service, told every version held.
fn connect(socket: &std::path::Path, holds: &Holds) -> std::io::Result<Connection> {
    let connection = Connection::connect(socket)?;
    connection.set_timeout(ANSWER)?;
    for (id, version_code) in holds.versions() {
        ask(
            &connection,
            &Request::Hold {
                id,
                version_code,
                uid: None,
            },
        )?;
    }
    Ok(connection)
}

/// Sends `request` and reads its answer; a refusal is said, a broken connection is an error.
fn ask(connection: &Connection, request: &Request) -> std::io::Result<()> {
    connection.send(request)?;
    match connection.receive::<serde_json::Value>()? {
        Some(Some(serde_json::Value::String(answer))) => {
            if answer == "rolledBack" {
                eprintln!("[session] the install service rolled back: {request:?}");
            }
            Ok(())
        }
        Some(Some(other)) => {
            eprintln!("[session] the install service answered {other} to {request:?}");
            Ok(())
        }
        Some(None) => Err(std::io::Error::other("an answer that is not JSON")),
        None => Err(std::io::Error::other("the service closed the connection")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyos_install::socket::Listener;

    /// A service that answers every request and records it, as the install service would.
    fn service(path: &std::path::Path) -> mpsc::Receiver<(usize, Request)> {
        let listener = Listener::bind(path, 0o600).unwrap();
        let (seen, heard) = mpsc::channel();
        std::thread::spawn(move || {
            for number in 0.. {
                let Ok(connection) = listener.accept() else {
                    return;
                };
                while let Ok(Some(Some(request))) = connection.receive::<Request>() {
                    let answer = match &request {
                        Request::Hold { .. } => "held",
                        Request::Release { .. } => "released",
                        _ => "noted",
                    };
                    let _ = connection.send(&answer);
                    let _ = seen.send((number, request));
                }
            }
        });
        heard
    }

    fn next(heard: &mpsc::Receiver<(usize, Request)>) -> (usize, String) {
        let (connection, request) = heard.recv_timeout(Duration::from_secs(10)).unwrap();
        (connection, format!("{request:?}"))
    }

    #[test]
    fn versions_are_held_while_they_run_and_noted_once_shown_and_exited() {
        let dir =
            std::env::temp_dir().join(format!("oxyos-session-installd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("socket");
        let heard = service(&socket);
        let installd = Installd::start(socket);
        let hold = |id: &str| Note::Hold {
            id: id.into(),
            version_code: 12,
        };
        installd.note(hold("so.oxy.inbox"));
        installd.note(hold("so.oxy.inbox"));
        installd.note(Note::Shown {
            id: "so.oxy.inbox".into(),
            version_code: 12,
        });
        installd.note(Note::Exited {
            id: "so.oxy.inbox".into(),
            version_code: 12,
            shown: true,
        });
        installd.note(Note::Exited {
            id: "so.oxy.inbox".into(),
            version_code: 12,
            shown: false,
        });
        let said: Vec<(usize, String)> = (0..6).map(|_| next(&heard)).collect();
        let hold = r#"Hold { id: "so.oxy.inbox", version_code: 12, uid: None }"#;
        assert_eq!(
            said,
            [
                (0, hold.to_string()),
                (0, hold.to_string()),
                (
                    0,
                    r#"Shown { id: "so.oxy.inbox", version_code: 12, uid: None }"#.to_string()
                ),
                (
                    0,
                    r#"Exited { id: "so.oxy.inbox", version_code: 12, shown: true, uid: None }"#
                        .to_string()
                ),
                // The second process's exit lets the last hold go.
                (
                    0,
                    r#"Exited { id: "so.oxy.inbox", version_code: 12, shown: false, uid: None }"#
                        .to_string()
                ),
                (
                    0,
                    r#"Release { id: "so.oxy.inbox", version_code: 12, uid: None }"#.to_string()
                ),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_the_service_notes_are_dropped_and_the_session_goes_on() {
        let installd = Installd::start(PathBuf::from("/nonexistent/oxyos/installd/socket"));
        installd.note(Note::Hold {
            id: "so.oxy.inbox".into(),
            version_code: 1,
        });
        installd.note(Note::Shown {
            id: "so.oxy.inbox".into(),
            version_code: 1,
        });
    }
}
