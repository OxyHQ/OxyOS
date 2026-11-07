//! `oxyos-accountd`, the system service that holds the platform credential of every Oxy account
//! set up on the machine ([ADR 0020](../../../docs/adr/0020-oxy-account-login.mdx), decisions 3
//! and 6). It runs as root, because it makes users and sets their verifiers, and nothing else runs
//! with it; `oxyos-accountd.service` confines it to what that needs.
//!
//! - Its own socket (`oxyos-account`'s `ipc`): setting accounts up for the setup client, listing
//!   them and checking a password online for the login screen and the login program, telling the
//!   login screen where Oxy is. Only root and the login program's user may.
//! - The session socket (`oxyos-account`'s `session`): the session program registers what it
//!   launches and checks the lock's passwords online; the applications it launched get the
//!   platform's Oxy session (snapshots, tokens) and Settings removes the account, or erases the
//!   machine ([ADR 0054](../../../docs/adr/0054-recovery-and-erasing.mdx), `oxyos-account`'s
//!   `erase`): an administrator's own password, checked by PAM under `oxyos-accountd`
//!   ([`passwords`]), `oxyos-erase.service` started to arm it, then the machine restarted.
//!
//! The first account set up on a machine is its administrator (a member of `sudo`); at its start
//! the service makes the enrolled account with the lowest uid one where accounts are enrolled and
//! none is (a machine set up before administrators existed).
//!
//! Its configuration is its unit's environment:
//! - `OXYOS_OXY_API`: Oxy's API, `https://api.oxy.so` by default (plain HTTP only to this
//!   machine, for a development server);
//! - `OXYOS_OXY_CLIENT_ID`: OxyOS's public client id, which the platform's sign-ins name; without
//!   it nothing is set up (`notConfigured`) and passwords are not checked online;
//! - `OXYOS_SESSION_PROGRAM`: the session program, `/usr/bin/oxyos-session` by default, the one
//!   program that may register launches;
//! - `OXYOS_LOGIN_PROGRAM`: the login program, `/usr/libexec/oxyos/oxyos-login` by default, whose
//!   root process must be the registering session program's parent.
//!
//! `oxyos-accountd [--state DIRECTORY] [--pictures DIRECTORY] [--socket PATH]
//! [--session-socket PATH] [--api URL] [--enroller USER]`:
//! - `--state`: the enrolled accounts, `/var/lib/oxyos/accounts` by default;
//! - `--pictures`: the accounts' pictures, `/var/lib/oxyos/pictures` by default;
//! - `--socket`: its own socket, `/run/oxyos/accountd/socket` by default;
//! - `--session-socket`: the session socket, `/run/oxyos/accountd/session` by default;
//! - `--api`: Oxy's API, as `OXYOS_OXY_API` (the flag wins);
//! - `--enroller`: the login program's user, `oxyos-login` by default, read from `/etc/passwd` at
//!   each connection; while it has no entry, only root may enroll.
//!
//! It logs what each request came to (the account's user, the reason it was refused) on standard
//! error, the journal's; never a password, a secret, a code or a token.

mod passwords;

use oxyos_account::api::{HttpApi, PlatformClient};
use oxyos_account::erase::{Eraser, Systemctl};
use oxyos_account::ipc::{self, Enrollers, Listener, Outcome, Peer};
use oxyos_account::platform::Platform;
use oxyos_account::session::{self, Event, Kind, Registrars, Sessions};
use oxyos_account::setup::Setup;
use oxyos_account::store::{Store, PICTURES_DIRECTORY, STATE_DIRECTORY};
use oxyos_account::users::{passwd_entry, SystemUsers};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Conversations at once on its own socket.
const MAX_CONVERSATIONS: usize = 4;
/// Refusals of peers that may not enroll at once, each at most `ipc::REFUSAL_WAIT` long.
const MAX_REFUSALS: usize = 4;
/// Applications' connections at once, every user's (each user has at most
/// `session::MAX_APPLICATION_CONNECTIONS`; registrars are counted apart).
const MAX_SESSION_CONNECTIONS: usize = 64;
/// Refusals on the session socket at once.
const MAX_SESSION_REFUSALS: usize = 4;

struct Options {
    state: PathBuf,
    pictures: PathBuf,
    socket: PathBuf,
    session_socket: PathBuf,
    api: String,
    client_id: Option<String>,
    session_program: PathBuf,
    login_program: PathBuf,
    enroller: String,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn options() -> Result<Options, String> {
    let mut options = Options {
        state: STATE_DIRECTORY.into(),
        pictures: PICTURES_DIRECTORY.into(),
        socket: ipc::SOCKET.into(),
        session_socket: session::SOCKET.into(),
        api: env("OXYOS_OXY_API").unwrap_or_else(|| oxyos_account::api::OXY_API.into()),
        client_id: env("OXYOS_OXY_CLIENT_ID"),
        session_program: env("OXYOS_SESSION_PROGRAM")
            .unwrap_or_else(|| session::SESSION_PROGRAM.into())
            .into(),
        login_program: env("OXYOS_LOGIN_PROGRAM")
            .unwrap_or_else(|| session::LOGIN_PROGRAM.into())
            .into(),
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
            "--pictures" => options.pictures = value()?.into(),
            "--socket" => options.socket = value()?.into(),
            "--session-socket" => options.session_socket = value()?.into(),
            "--api" => options.api = value()?,
            "--enroller" => options.enroller = value()?,
            _ => {
                return Err(format!(
                    "unknown argument {argument}\n\
                     usage: oxyos-accountd [--state DIRECTORY] [--pictures DIRECTORY] \
                     [--socket PATH] [--session-socket PATH] [--api URL] [--enroller USER]"
                ))
            }
        }
    }
    if !options.session_program.is_absolute() || !options.login_program.is_absolute() {
        return Err("OXYOS_SESSION_PROGRAM and OXYOS_LOGIN_PROGRAM must be absolute paths".into());
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
        Outcome::Listed { accounts, broken } => {
            eprintln!("oxyos-accountd: {who} listed {accounts} accounts");
            for file in broken {
                eprintln!("oxyos-accountd: not an account, left out: {file}");
            }
        }
        Outcome::Unlisted(error) => eprintln!("oxyos-accountd: {who}: the accounts: {error}"),
        Outcome::Platform => {}
        Outcome::Checked { user, checked } => {
            eprintln!("oxyos-accountd: {who} checked {user}'s password: {checked:?}")
        }
    }
}

fn session_log(uid: u32, event: &Event) {
    match event {
        Event::Registered { id, pid } => {
            eprintln!("oxyos-accountd: uid {uid}'s session launched {id} (pid {pid})")
        }
        Event::RegistrationRefused { id, reason } => {
            eprintln!("oxyos-accountd: uid {uid}'s session: {id} not registered: {reason:?}")
        }
        Event::Checked(checked) => {
            eprintln!("oxyos-accountd: uid {uid}'s lock checked its password: {checked:?}")
        }
        Event::Removed => eprintln!("oxyos-accountd: uid {uid}'s Settings removed its account"),
        Event::ErasePreviewed => {
            eprintln!("oxyos-accountd: uid {uid}'s Settings asked what erasing would do")
        }
        Event::Erasing {
            signed_out,
            not_signed_out,
        } => eprintln!(
            "oxyos-accountd: uid {uid}'s Settings erased this machine: {signed_out} accounts \
             signed out online, {not_signed_out} not; restarting"
        ),
        Event::EraseRefused(reason) => {
            eprintln!("oxyos-accountd: uid {uid}'s Settings: the erase refused: {reason:?}")
        }
        Event::Refused(_) => {
            eprintln!("oxyos-accountd: uid {uid} on the session socket: not permitted")
        }
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

/// A slot among `limit`, taken until the returned guard is dropped.
struct Slot(Arc<AtomicUsize>);

impl Slot {
    /// No slot of a shared pool: the peer is bounded otherwise.
    fn none() -> Self {
        Self(Arc::new(AtomicUsize::new(1)))
    }

    fn take(open: &Arc<AtomicUsize>, limit: usize) -> Option<Self> {
        if open.fetch_add(1, Ordering::SeqCst) >= limit {
            open.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Self(open.clone()))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

type Service = Platform<HttpApi, SystemUsers>;

/// The session socket: each connection classified by the kernel's word on its peer, then served
/// on a thread of its own.
fn sessions(listener: Listener, sessions: Arc<Sessions<HttpApi, SystemUsers>>) {
    let connections = Arc::new(AtomicUsize::new(0));
    let refusals = Arc::new(AtomicUsize::new(0));
    loop {
        let connection = match listener.accept() {
            Ok(connection) => connection,
            Err(error) => {
                eprintln!("oxyos-accountd: accepting a session's connection: {error}");
                continue;
            }
        };
        let uid = connection.peer().map_or(u32::MAX, |peer| peer.uid);
        let kind = sessions.classify(&connection);
        // A registrar is bounded by its user's few (session::MAX_REGISTRARS), never by the
        // applications': no application keeps a lock from its check.
        let slot = match kind {
            Kind::Refused => Slot::take(&refusals, MAX_SESSION_REFUSALS),
            Kind::Registrar { .. } => Some(Slot::none()),
            Kind::Application(_) => Slot::take(&connections, MAX_SESSION_CONNECTIONS),
        };
        let Some(slot) = slot else {
            eprintln!(
                "oxyos-accountd: too many session connections at once: one closed unanswered"
            );
            sessions.unserved(kind);
            continue;
        };
        let served = sessions.clone();
        let spawned = std::thread::Builder::new()
            .name("oxyos-accountd-session".into())
            .spawn(move || {
                served.serve(&connection, kind, |event| session_log(uid, event));
                drop(slot);
            });
        if let Err(error) = spawned {
            eprintln!("oxyos-accountd: a session's connection cannot be served: {error}");
        }
    }
}

fn run() -> Result<(), String> {
    let options = options()?;
    if !rustix::process::geteuid().is_root() {
        return Err("it runs as root: it makes users and sets their verifiers".into());
    }
    let api = HttpApi::new(&options.api)?;
    let store = Store::open(&options.state)
        .map_err(|error| format!("{}: {error}", options.state.display()))?
        .with_pictures(&options.pictures);
    let mut setup = Setup::new(api, SystemUsers::new(), store);
    match &options.client_id {
        Some(client_id) => {
            setup = setup.with_platform(PlatformClient {
                client_id: client_id.clone(),
            })
        }
        None => eprintln!(
            "oxyos-accountd: OXYOS_OXY_CLIENT_ID is not set: no account can be set up, and \
             passwords are not checked online"
        ),
    }
    match setup.ensure_administrator() {
        Ok(Some(user)) => eprintln!(
            "oxyos-accountd: no enrolled account was an administrator: {user}, the first set up, \
             is now"
        ),
        Ok(None) => {}
        Err(error) => eprintln!("oxyos-accountd: the machine has no administrator: {error}"),
    }
    let platform: Arc<Service> = Arc::new(Platform::new(setup));
    let listener = Listener::bind(&options.socket)
        .map_err(|error| format!("{}: {error}", options.socket.display()))?;
    let session_listener = Listener::bind(&options.session_socket)
        .map_err(|error| format!("{}: {error}", options.session_socket.display()))?;
    eprintln!(
        "oxyos-accountd: listening on {} and {}, accounts in {}, Oxy at {}, sessions run {}",
        options.socket.display(),
        options.session_socket.display(),
        options.state.display(),
        options.api,
        options.session_program.display()
    );
    let session_service = Arc::new(Sessions::new(
        platform.clone(),
        Registrars::new(
            options.session_program.clone(),
            options.login_program.clone(),
        ),
        Eraser::new(
            Box::new(Systemctl::new()),
            Box::new(passwords::PamPasswords::new()),
        ),
    ));
    std::thread::Builder::new()
        .name("oxyos-accountd-sessions".into())
        .spawn(move || sessions(session_listener, session_service))
        .map_err(|error| format!("the session socket: {error}"))?;
    // Conversations, and refusals of peers that may not enroll, each have their own few threads:
    // peers that may not enroll cannot keep those that may out. A connection past them is closed
    // unanswered (its client sees the connection end, and nothing is set up).
    let conversations = Arc::new(AtomicUsize::new(0));
    let refusals = Arc::new(AtomicUsize::new(0));
    let api: Arc<str> = options.api.clone().into();
    loop {
        let connection = match listener.accept() {
            Ok(connection) => connection,
            Err(error) => {
                eprintln!("oxyos-accountd: accepting a connection: {error}");
                continue;
            }
        };
        let enrollers = enrollers(&options.enroller);
        let allowed = match connection.peer() {
            Ok(peer) => enrollers.allow(peer.uid),
            Err(error) => {
                eprintln!("oxyos-accountd: a connection's peer: {error}");
                continue;
            }
        };
        let slot = if allowed {
            Slot::take(&conversations, MAX_CONVERSATIONS)
        } else {
            Slot::take(&refusals, MAX_REFUSALS)
        };
        let Some(slot) = slot else {
            eprintln!("oxyos-accountd: too many connections at once: one closed unanswered");
            continue;
        };
        let platform = platform.clone();
        let api = api.clone();
        let spawned = std::thread::Builder::new()
            .name("oxyos-accountd-conversation".into())
            .spawn(move || {
                // serve checks the peer itself too, and refuses it if it may not enroll.
                if let Err(error) = ipc::serve(&connection, &platform, &api, enrollers, log) {
                    eprintln!("oxyos-accountd: a conversation ended: {error}");
                }
                drop(slot);
            });
        if let Err(error) = spawned {
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
