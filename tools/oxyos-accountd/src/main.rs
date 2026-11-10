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
//! - The accounts' keychains (`oxyos-account`'s `keychain`, ADR 0050): unlocked by the login
//!   program and the session's lock with the account's password, locked with the session; AutoFill
//!   for the session, the vault for Settings, synced with Oxy with OxyOS's own token. Their keys are
//!   locked in memory: a keychain whose keys cannot be stays locked (the unit's `LimitMEMLOCK`).
//! - The accounts' PINs (`oxyos-account`'s `pin`): set at setup and by Settings, checked for PAM
//!   by `oxyos-pin-check` (this package's other program, which `pam_exec` runs for the PIN
//!   services), offline, with their own throttle and lockout.
//! - The person at the seat ([`seat`], [ADR 0066](../../../docs/adr/0066-quality-of-service.mdx),
//!   decision 8): the user slice of seat0's active session weighs 400 against every other user's
//!   100, set through systemd as logind's seat changes, on a thread of its own; the memory its
//!   session's applications claim (told on the session socket) is carried up its user manager,
//!   its slice and the users' slice while it holds the seat.
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
//! - `OXYOS_OXY_CLOUD`: Oxy's cloud, where an account's avatar is read from to be its picture,
//!   `https://cloud.oxy.so` by default (a development server's may be plain HTTP to this machine);
//! - `OXYOS_SESSION_PROGRAM`: the session program, `/usr/bin/oxyos-session` by default, the one
//!   program that may register launches;
//! - `OXYOS_LOGIN_PROGRAM`: the login program, `/usr/libexec/oxyos/oxyos-login` by default, whose
//!   root process must be the registering session program's parent.
//!
//! `oxyos-accountd [--state DIRECTORY] [--keychains DIRECTORY] [--pictures DIRECTORY]
//! [--socket PATH] [--session-socket PATH] [--api URL] [--enroller USER]`:
//! - `--state`: the enrolled accounts, `/var/lib/oxyos/accounts` by default;
//! - `--keychains`: the accounts' keychains, `/var/lib/oxyos/keychain` by default (0700, root's);
//! - `--pictures`: the accounts' pictures, `/var/lib/oxyos/pictures` by default;
//! - `--socket`: its own socket, `/run/oxyos/accountd/socket` by default;
//! - `--session-socket`: the session socket, `/run/oxyos/accountd/session` by default;
//! - `--api`: Oxy's API, as `OXYOS_OXY_API` (the flag wins);
//! - `--enroller`: the login program's user, `oxyos-login` by default, read from `/etc/passwd` at
//!   each connection; while it has no entry, only root may enroll.
//!
//! `oxyos-accountd --apply-seat-weights USER_SLICES SEAT` gives a tree of slices that is not
//! systemd's the seat's weights ([`seat::apply_to_tree`]): SEAT, a directory of USER_SLICES,
//! the seat's holder's, every other `*.slice` there the default, each printed with its weight.
//! The saturation benchmark's (`perf.py saturation`, S7) weighs its users so; it needs no root.
//! `oxyos-accountd --apply-memory-claim USER_SLICES SEAT BYTES` gives such a tree the protections
//! a claim of BYTES carries up ([`seat::apply_memory_claim_to_tree`]: `memory.low` of
//! SEAT/user@.service, SEAT and USER_SLICES), each printed with its bytes (S6).
//!
//! It logs what each request came to (the account's user, the reason it was refused) on standard
//! error, the journal's; never a password, a secret, a code or a token.

mod passwords;
mod seat;
#[cfg(test)]
mod unit_sandbox;

use oxyos_account::api::{Attestation, HttpApi, PlatformClient};
use oxyos_account::erase::{Eraser, Systemctl};
use oxyos_account::ipc::{self, Enrollers, Listener, Outcome, Peer};
use oxyos_account::keychain::service::{self as keychain, KeychainService, Memory, TokenSource};
use oxyos_account::platform::{Platform, TokenReason};
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
    keychains: PathBuf,
    pictures: PathBuf,
    socket: PathBuf,
    session_socket: PathBuf,
    api: String,
    cloud: String,
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
        keychains: keychain::STATE_DIRECTORY.into(),
        pictures: PICTURES_DIRECTORY.into(),
        socket: ipc::SOCKET.into(),
        session_socket: session::SOCKET.into(),
        api: env("OXYOS_OXY_API").unwrap_or_else(|| oxyos_account::api::OXY_API.into()),
        cloud: env("OXYOS_OXY_CLOUD").unwrap_or_else(|| oxyos_account::api::OXY_CLOUD.into()),
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
            "--keychains" => options.keychains = value()?.into(),
            "--pictures" => options.pictures = value()?.into(),
            "--socket" => options.socket = value()?.into(),
            "--session-socket" => options.session_socket = value()?.into(),
            "--api" => options.api = value()?,
            "--enroller" => options.enroller = value()?,
            _ => {
                return Err(format!(
                    "unknown argument {argument}\n\
                     usage: oxyos-accountd [--state DIRECTORY] [--keychains DIRECTORY] \
                     [--pictures DIRECTORY] \
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
        Outcome::Interrupted(error) => {
            eprintln!("oxyos-accountd: {who}: setup failed, its sign-in kept for a retry: {error}")
        }
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
        Outcome::KeychainUnlocked { user, state } => {
            eprintln!("oxyos-accountd: {who} unlocked {user}'s keychain: {state:?}")
        }
        Outcome::PinChecked { user, result } => {
            eprintln!("oxyos-accountd: {who} checked {user}'s PIN: {result:?}")
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
        Event::Saved { id } => {
            eprintln!("oxyos-accountd: uid {uid}'s {id}: a login saved, as the person said")
        }
        Event::NotSaved { id } => {
            eprintln!("oxyos-accountd: uid {uid}'s {id}: not saved, as the person said")
        }
        Event::SaveRefused { id, reason } => {
            eprintln!("oxyos-accountd: uid {uid}'s {id}: not saved: {reason:?}")
        }
        Event::Refused(_) => {
            eprintln!("oxyos-accountd: uid {uid} on the session socket: not permitted")
        }
        Event::LocalChanged {
            change,
            refused: None,
        } => eprintln!("oxyos-accountd: uid {uid}'s Settings: {change}"),
        Event::LocalChanged {
            change,
            refused: Some(reason),
        } => eprintln!("oxyos-accountd: uid {uid}'s Settings: {change} refused: {reason:?}"),
    }
}

/// What this machine calls itself as a member of a vault: its host name, else `OxyOS`.
fn machine_name() -> String {
    let name = std::fs::read_to_string("/etc/hostname").unwrap_or_default();
    let name = name.trim();
    if name.is_empty() || name.len() > 256 {
        "OxyOS".into()
    } else {
        name.to_owned()
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

/// `path` as a directory of the service's own, 0700: made if it is not there, refused if it is
/// anything else (`Registrars` reads nothing from a directory that is not).
fn own_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Err(error) if error.kind() != std::io::ErrorKind::AlreadyExists => return Err(error),
        _ => {}
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.uid() != rustix::process::geteuid().as_raw() {
        return Err(std::io::Error::other(
            "not a directory of the service's own",
        ));
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

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
    let api = HttpApi::new(&options.api)?.with_cloud(&options.cloud)?;
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
    // Each account's user named as its account is (passwd's GECOS): users set up before the
    // service named them are named now.
    if let Err(error) = setup.name_users() {
        eprintln!("oxyos-accountd: the accounts' users are not named: {error}");
    }
    let platform: Arc<Service> = Arc::new(Platform::new(setup));
    // The accounts' keychains, synced with OxyOS's own token for each account.
    let tokens = Arc::downgrade(&platform);
    let token: TokenSource = Box::new(move |uid, refresh| {
        let reason = if refresh {
            TokenReason::Response401
        } else {
            TokenReason::Preflight
        };
        tokens
            .upgrade()?
            .token(uid, &Attestation::OwnPlatform, reason)
            .map(|token| token.access_token)
    });
    let keychains = KeychainService::new(
        HttpApi::new(&options.api)?,
        options.keychains.clone(),
        machine_name(),
        token,
        Memory::Locked,
    );
    platform.set_keychains(keychains);
    // Each account's picture read where it is missing or out of date (accounts set up before
    // pictures were kept), on a thread of its own: nothing waits for Oxy's cloud, and offline the
    // last one read is shown.
    let pictures = Arc::clone(&platform);
    std::thread::spawn(move || pictures.refresh_pictures());
    // The person at the seat above other users, followed on a thread of its own, with the memory
    // the sessions' applications claim (told on the session socket, below).
    let (claimed, claims) = async_channel::unbounded();
    std::thread::Builder::new()
        .name("oxyos-accountd-seat".into())
        .spawn(move || seat::run(claims, |line| eprintln!("oxyos-accountd: {line}")))
        .map_err(|error| format!("following the seat: {error}"))?;
    let listener = Listener::bind(&options.socket)
        .map_err(|error| format!("{}: {error}", options.socket.display()))?;
    let session_listener = Listener::bind(&options.session_socket)
        .map_err(|error| format!("{}: {error}", options.session_socket.display()))?;
    eprintln!(
        "oxyos-accountd: listening on {} and {}, accounts in {}, keychains in {}, Oxy at {}, \
         sessions run {}",
        options.socket.display(),
        options.session_socket.display(),
        options.state.display(),
        options.keychains.display(),
        options.api,
        options.session_program.display()
    );
    // The registrars verified this boot, beside the session socket (session::VERIFIED): a session
    // running when the service starts again is known by them, though an upgrade replaced its
    // program since.
    let verified = options
        .session_socket
        .parent()
        .unwrap_or(Path::new("/"))
        .join("registrars");
    if let Err(error) = own_directory(&verified) {
        eprintln!(
            "oxyos-accountd: {}: {error}: a session running when the service starts again is \
             not known if an upgrade replaced its program",
            verified.display()
        );
    }
    let session_service = Arc::new(
        Sessions::new(
        platform.clone(),
        Registrars::new(
            options.session_program.clone(),
            options.login_program.clone(),
        )
        .remembering(verified),
        Eraser::new(
            Box::new(Systemctl::new()),
            Box::new(passwords::PamPasswords::new()),
        ),
        )
        .with_memory_claims(Box::new(move |claim| {
            // Unbounded: the seat's thread takes them as they come.
            let _ = claimed.try_send(claim);
        })),
    );
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

/// `--apply-memory-claim USER_SLICES SEAT BYTES` (the module's doc).
fn apply_memory_claim(arguments: &[String]) -> ExitCode {
    let [user_slices, seat, bytes] = arguments else {
        eprintln!("usage: oxyos-accountd --apply-memory-claim USER_SLICES SEAT BYTES");
        return ExitCode::from(2);
    };
    let Ok(bytes) = bytes.parse::<u64>() else {
        eprintln!("oxyos-accountd: {bytes}: not a number of bytes");
        return ExitCode::from(2);
    };
    match seat::apply_memory_claim_to_tree(Path::new(user_slices), seat, bytes) {
        Ok(applied) => {
            for (group, low) in applied {
                println!("{group} {low}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("oxyos-accountd: {user_slices}: {error}");
            ExitCode::FAILURE
        }
    }
}

/// `--apply-seat-weights USER_SLICES SEAT` (the module's doc).
fn apply_seat_weights(arguments: &[String]) -> ExitCode {
    let [user_slices, seat] = arguments else {
        eprintln!("usage: oxyos-accountd --apply-seat-weights USER_SLICES SEAT");
        return ExitCode::from(2);
    };
    match seat::apply_to_tree(Path::new(user_slices), seat) {
        Ok(applied) => {
            for (slice, weight) in applied {
                println!("{slice} {weight}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("oxyos-accountd: {user_slices}: {error}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.first().map(String::as_str) {
        Some("--apply-seat-weights") => return apply_seat_weights(&arguments[1..]),
        Some("--apply-memory-claim") => return apply_memory_claim(&arguments[1..]),
        _ => {}
    }
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("oxyos-accountd: {error}");
            ExitCode::FAILURE
        }
    }
}
