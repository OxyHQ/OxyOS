//! The session socket, `/run/oxyos/accountd/session` ([ADR 0020](../../docs/adr/0020-oxy-account-login.mdx),
//! decision 6, step 5): how a signed-in session's applications get the platform's Oxy session
//! (Oxy ADR 0058) without ever holding its credential.
//!
//! - **Transport:** `SOCK_SEQPACKET`, one JSON message per packet, as the service's own socket.
//!   Mode 0666: every peer is known by the kernel (`SO_PEERCRED` and `SO_PEERPIDFD`), never by
//!   what it sends. A peer that is neither of the two below is answered
//!   `{"refused": {"reason": "notPermitted"}}`, once its first message is read, and nothing else.
//! - **The registrar** is the session program the login program started: its uid is an enrolled
//!   account's user; the program its pidfd's process runs (`/proc/<pid>/exe`, the file itself, not
//!   a path) is `OXYOS_SESSION_PROGRAM`'s, owned by root and writable by no group or other; and its
//!   parent is a root process running `OXYOS_LOGIN_PROGRAM`'s file (the login program's worker,
//!   which starts the session with a clean environment). The session program run by the user
//!   itself (from a terminal, with `LD_PRELOAD`) is no registrar: its parent is the user's. It says what it
//!   launched, the launched process handed over as a pidfd (`SCM_RIGHTS`), which must be its own
//!   child and its user's:
//!
//!   ```text
//!   → {"register": {"desktopEntryId": "so.oxy.dropsea", "system": true}}   + the child's pidfd
//!   ← "registered" | {"refused": {"reason"}}
//!   → {"check": {"password", "local": "accepted" | "rejected"}}          the lock's online check
//!   ← {"checked": {"result", "reason"?}}                                  for its own account
//!   ```
//!
//!   Accepted, its account's keychain is unlocked with that password; the registrar also asks
//!   AutoFill's requests and locks the keychain when the session locks (`crate::keychain`). The
//!   session's last registrar connection closed, its keychain locks.
//!
//!   A registration lives while its process does (the service watches the pidfd) and while the
//!   registrar's connection stays open: the session keeps one for its life, and closing it drops
//!   every registration made on it.
//! - **An application** is a process whose pidfd is a live registration's, of the registrar's
//!   user. Anything else (another user, a process nobody registered, a child of a launched one)
//!   is refused. It may ask, any number of times on one connection:
//!
//!   ```text
//!   → "snapshot"                         ← {"snapshot": Snapshot}
//!   → "subscribe"                        ← {"snapshot": Snapshot}, now and on every change,
//!                                          until the client closes (nothing else is answered)
//!   → {"token": {"reason": "boot" | "preflight" | "response-401"}}
//!                                        ← {"token": {"accessToken", "expiresAt"}} | {"token": null}
//!   → {"remove": {}}                     ← "removed" | {"refused": {"reason"}}
//!   ```
//!
//!   `Snapshot` is the SDK's `OxyHostSnapshot`: `{"status": "signed_in" | "signed_out" |
//!   "unavailable", "revision", "account": {"id", "handle", "displayName"?, "avatar"?} | null,
//!   "online"}`. A token is the application's own (Settings and the shell, registered as system
//!   applications, are OxyOS's; any other by its desktop entry id); null offline, signed out or
//!   refused. `remove` is Settings' alone, registered as a system application: decision 5's Remove
//!   of the peer's own account; so are the keychain's requests (`crate::keychain`), which any
//!   other application is refused `notPermitted`.
//! - **Limits,** as the service's own socket: every peer waits at most [`IDLE`] for its next
//!   message but a registrar or a subscriber, at most [`MAX_REGISTRARS`] registrars and
//!   [`MAX_APPLICATION_CONNECTIONS`] application connections per user (so one user's applications
//!   never keep another's lock from its check),
//!   [`MAX_REGISTRATIONS`] registrations per registrar and [`MAX_SUBSCRIPTIONS`] subscriptions per
//!   registration. Every answer follows the message it answers, and no connection is closed with
//!   a message unread.

use crate::api::OxyApi;
use crate::ipc::{Connection, MAX_MESSAGE};
use crate::keychain::{
    self, AuditView, EnrollmentView, Field, ItemInput, ItemKind, ItemSummary, ItemView, JoinMethod,
    MemberView, Status, Suggestion,
};
use crate::pidfd::{self, Program};
use crate::platform::{self, Checked, Local, Platform, Snapshot, Token, TokenReason};
use crate::secret::Secret;
use crate::store::Enrollment;
use crate::users::Users;
use rustix::event::{poll, PollFd, PollFlags};
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Where the service listens for sessions and their applications.
pub const SOCKET: &str = "/run/oxyos/accountd/session";

/// The session program, unless `OXYOS_SESSION_PROGRAM` names another.
pub const SESSION_PROGRAM: &str = "/usr/bin/oxyos-session";

/// How long an application's connection waits for its next message.
pub const IDLE: Duration = Duration::from_secs(60);
/// Registrar connections of one user at once.
pub const MAX_REGISTRARS: usize = 4;
/// Application connections of one user at once.
pub const MAX_APPLICATION_CONNECTIONS: usize = 16;

/// The login program, unless `OXYOS_LOGIN_PROGRAM` names another: the registrar's parent.
pub const LOGIN_PROGRAM: &str = "/usr/libexec/oxyos/oxyos-login";
/// Registrations of one registrar at once.
pub const MAX_REGISTRATIONS: usize = 512;
/// Subscriptions of one registration at once.
pub const MAX_SUBSCRIPTIONS: usize = 4;

const _: () = assert!(MAX_MESSAGE >= 4096);

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Request {
    #[serde(rename_all = "camelCase")]
    Register {
        desktop_entry_id: String,
        #[serde(default)]
        system: bool,
    },
    Check {
        password: Secret,
        local: Local,
    },
    Snapshot,
    Subscribe,
    Token {
        reason: TokenReason,
    },
    Remove {},
    // The keychain (ADR 0050 step 2, `crate::keychain`): the registrar's.
    #[serde(rename_all = "camelCase")]
    AutofillSuggest {
        desktop_entry_id: String,
        field: Field,
    },
    #[serde(rename_all = "camelCase")]
    AutofillFill {
        desktop_entry_id: String,
        #[serde(rename = "ref")]
        reference: String,
        field: Field,
    },
    AutofillGenerate {},
    KeychainLock {},
    // Settings'.
    KeychainStatus {},
    KeychainCreate {},
    #[serde(rename_all = "camelCase")]
    KeychainConfirmRecoveryKey {
        recovery_key: Secret,
    },
    #[serde(rename_all = "camelCase")]
    KeychainJoin {
        method: JoinMethod,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recovery_key: Option<Secret>,
    },
    KeychainEnrollments {},
    #[serde(rename_all = "camelCase")]
    KeychainApprove {
        enrollment_id: String,
        #[serde(default)]
        confirm: bool,
    },
    #[serde(rename_all = "camelCase")]
    KeychainDeny {
        enrollment_id: String,
    },
    KeychainMembers {},
    #[serde(rename_all = "camelCase")]
    KeychainRemoveMember {
        member_id: String,
    },
    KeychainVerify {
        password: Secret,
    },
    KeychainList {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query: Option<Secret>,
        #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
        kind: Option<ItemKind>,
    },
    #[serde(rename_all = "camelCase")]
    KeychainReveal {
        item_id: String,
    },
    KeychainPut {
        item: ItemInput,
    },
    #[serde(rename_all = "camelCase")]
    KeychainDelete {
        item_id: String,
    },
    KeychainAudit {},
    KeychainRekey {
        previous: Secret,
        current: Secret,
    },
    KeychainLeave {},
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    NotPermitted,
    InvalidRequest,
    TooMany,
    Failed,
    // The keychain's.
    /// The keychain is locked (the session is, or was never unlocked since the service started).
    Locked,
    /// Oxy cannot be reached.
    Offline,
    /// Oxy refused, or answered what does not verify: nothing of it was kept.
    Oxy,
    /// The account has no vault, or the machine is no member of it.
    NotMember,
    /// The account already has a vault (join it), or this machine is a member already.
    VaultExists,
    /// Another device removed this machine, or the vault was reset.
    Removed,
    /// The keybag is sealed under a password the machine no longer has (`keychainRekey`).
    NeedsPreviousPassword,
    /// Settings must verify the person first (`keychainVerify`).
    VerificationRequired,
    WrongPassword,
    /// Too many wrong passwords: try again later.
    Throttled,
    /// Not the recovery key shown, or not the vault's.
    RecoveryKeyMismatch,
    /// No such item, enrollment, member or suggestion (or its `ref` died with a lock).
    NotFound,
    /// The enrollment was denied.
    Denied,
    /// The enrollment's ten minutes are over.
    Expired,
    /// Another device wrote first: try again.
    Conflict,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Answer {
    Registered,
    Refused {
        reason: Reason,
    },
    Checked(Checked),
    Snapshot(Snapshot),
    Token(Option<Token>),
    Removed,
    // The keychain's (`crate::keychain`).
    AutofillSuggestions(Vec<Suggestion>),
    AutofillCredential {
        username: Secret,
        password: Secret,
    },
    AutofillCode {
        code: Secret,
    },
    GeneratedPassword(Secret),
    KeychainLocked,
    KeychainStatus(Status),
    #[serde(rename_all = "camelCase")]
    KeychainRecoveryKey {
        recovery_key: Secret,
    },
    KeychainEnrollment(EnrollmentView),
    KeychainEnrollments(Vec<EnrollmentView>),
    KeychainApproved,
    KeychainDenied,
    KeychainMembers(Vec<MemberView>),
    KeychainMemberRemoved,
    KeychainVerified,
    KeychainItems(Vec<ItemSummary>),
    KeychainItem(ItemView),
    #[serde(rename_all = "camelCase")]
    KeychainSaved {
        item_id: String,
    },
    KeychainDeleted,
    KeychainAudit(AuditView),
    KeychainLeft,
}

impl Answer {
    pub fn refused(reason: Reason) -> Self {
        Answer::Refused { reason }
    }
}

/// Who may register: the session program, by its file.
#[derive(Clone, Debug)]
pub struct Registrars {
    pub program: PathBuf,
    /// The program that starts it, whose process is the registrar's parent.
    pub parent: PathBuf,
    /// Both programs' owner and the parent's user: root (the tests', their own user).
    pub owner: u32,
}

impl Registrars {
    pub fn new(program: PathBuf, parent: PathBuf) -> Self {
        Self {
            program,
            parent,
            owner: 0,
        }
    }

    /// Whether `pidfd`'s process runs the session program and its parent, of the owner's user,
    /// the login program; judged while it lives.
    fn runs_program(&self, pidfd: &OwnedFd) -> bool {
        let (Ok(program), Ok(parent)) = (Program::at(&self.program), Program::at(&self.parent))
        else {
            return false;
        };
        pidfd::while_alive(pidfd, |pid| {
            let running = Program::of_process(pid)?;
            let status = pidfd::status(pid)?;
            let started_by = Program::of_process(status.parent)?;
            let starter = pidfd::status(status.parent)?;
            Ok(running.is(&program, self.owner)
                && started_by.is(&parent, self.owner)
                && starter.uid == self.owner)
        })
        .is_some_and(|(_, verified)| verified)
    }
}

/// A process the session launched, while it runs and its registrar's connection is open.
pub struct Registration {
    pub desktop_entry_id: String,
    pub system: bool,
    pub uid: u32,
    pub pid: i32,
    pidfd: OwnedFd,
    registrar: u64,
    /// Written to once the registration has ended, for its subscribers.
    ended: OwnedFd,
    subscriptions: Mutex<usize>,
}

impl Registration {
    /// Whether its process is still the one registered.
    fn alive(&self) -> bool {
        matches!(pidfd::pid(self.pidfd.as_fd()), Ok(Some(pid)) if pid == self.pid)
    }
}

/// What a peer of the session socket is.
pub enum Kind {
    Registrar {
        id: u64,
        enrollment: Box<Enrollment>,
        pid: i32,
    },
    Application(Arc<Registration>),
    Refused,
}

/// The session socket's side of the service.
pub struct Sessions<A, U> {
    platform: Arc<Platform<A, U>>,
    registrars: Registrars,
    registrations: Mutex<Vec<Arc<Registration>>>,
    /// Registrar connections open, by uid.
    open_registrars: Mutex<Vec<u32>>,
    /// Application connections open, by uid.
    open_applications: Mutex<Vec<u32>>,
    next: AtomicU64,
}

/// Takes one of `uid`'s places among `open`, if it has fewer than `limit`.
fn take(open: &Mutex<Vec<u32>>, uid: u32, limit: usize) -> bool {
    let mut open = open.lock().unwrap_or_else(|p| p.into_inner());
    if open.iter().filter(|held| **held == uid).count() >= limit {
        return false;
    }
    open.push(uid);
    true
}

fn give_back(open: &Mutex<Vec<u32>>, uid: u32) {
    let mut open = open.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(index) = open.iter().position(|held| *held == uid) {
        open.remove(index);
    }
}

fn eventfd() -> io::Result<OwnedFd> {
    use rustix::event::EventfdFlags;
    Ok(rustix::event::eventfd(
        0,
        EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK,
    )?)
}

fn drain(fd: &OwnedFd) {
    let mut count = [0u8; 8];
    let _ = rustix::io::read(fd, &mut count);
}

/// What a connection's peer did, for the service's log: never a secret.
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    Registered { id: String, pid: i32 },
    RegistrationRefused { id: String, reason: Reason },
    Checked(Checked),
    Removed,
    Refused(Reason),
}

impl<A: OxyApi + Send + Sync + 'static, U: Users + Send + Sync + 'static> Sessions<A, U> {
    pub fn new(platform: Arc<Platform<A, U>>, registrars: Registrars) -> Self {
        Self {
            platform,
            registrars,
            registrations: Mutex::new(Vec::new()),
            open_registrars: Mutex::new(Vec::new()),
            open_applications: Mutex::new(Vec::new()),
            next: AtomicU64::new(1),
        }
    }

    pub fn platform(&self) -> &Arc<Platform<A, U>> {
        &self.platform
    }

    /// The live registrations (for the tests and the log).
    pub fn registered(&self) -> Vec<(String, i32)> {
        self.registrations
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|registration| (registration.desktop_entry_id.clone(), registration.pid))
            .collect()
    }

    /// What the peer of `connection` is, from the kernel's word on it.
    pub fn classify(&self, connection: &Connection) -> Kind {
        let (Ok(peer), Ok(pidfd)) = (connection.peer(), connection.peer_pidfd()) else {
            return Kind::Refused;
        };
        let Some(peer_pid) = pidfd::pid(pidfd.as_fd()).ok().flatten() else {
            return Kind::Refused;
        };
        // A registered application: its pidfd's pid is a live registration's, of its user. Both
        // pids are read again after: two live processes never share one.
        let registration = {
            let registrations = self.registrations.lock().unwrap_or_else(|p| p.into_inner());
            registrations
                .iter()
                .find(|registration| registration.pid == peer_pid && registration.uid == peer.uid)
                .cloned()
        };
        if let Some(registration) = registration {
            if registration.alive()
                && matches!(pidfd::pid(pidfd.as_fd()), Ok(Some(pid)) if pid == peer_pid)
                && take(
                    &self.open_applications,
                    peer.uid,
                    MAX_APPLICATION_CONNECTIONS,
                )
            {
                return Kind::Application(registration);
            }
            return Kind::Refused;
        }
        // The registrar: an enrolled account's user running the session program.
        let Ok(Some(enrollment)) = self.platform.enrollment_of(peer.uid) else {
            return Kind::Refused;
        };
        if !self.registrars.runs_program(&pidfd) {
            return Kind::Refused;
        }
        if !take(&self.open_registrars, peer.uid, MAX_REGISTRARS) {
            return Kind::Refused;
        }
        Kind::Registrar {
            id: self.next.fetch_add(1, Ordering::SeqCst),
            enrollment: Box::new(enrollment),
            pid: peer_pid,
        }
    }

    /// Serves `connection` as `kind` says, to its end. Each event is handed to `log`.
    pub fn serve(&self, connection: &Connection, kind: Kind, mut log: impl FnMut(&Event)) {
        match kind {
            Kind::Refused => {
                log(&Event::Refused(Reason::NotPermitted));
                let _ = refuse(connection);
            }
            Kind::Registrar {
                id,
                enrollment,
                pid,
            } => {
                let uid = enrollment.uid;
                if let Some(keychains) = self.platform.keychains() {
                    keychains.session_opened(uid, enrollment.user.as_str());
                }
                let served = self.serve_registrar(connection, id, &enrollment, pid, &mut log);
                self.drop_registrar(id);
                self.release_registrar(uid);
                // The session's last connection gone, its keychain locks (ADR 0050 decision 2).
                if let Some(keychains) = self.platform.keychains() {
                    let remaining = self
                        .open_registrars
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .iter()
                        .filter(|held| **held == uid)
                        .count();
                    keychains.session_closed(uid, remaining);
                }
                if let Err(error) = served {
                    eprintln!("oxyos-accountd: a session's connection ended: {error}");
                }
            }
            Kind::Application(registration) => {
                if let Err(error) = self.serve_application(connection, &registration, &mut log) {
                    if error.kind() != io::ErrorKind::WouldBlock {
                        eprintln!(
                            "oxyos-accountd: {}'s connection ended: {error}",
                            registration.desktop_entry_id
                        );
                    }
                }
                give_back(&self.open_applications, registration.uid);
            }
        }
    }

    /// A peer classified and then not served (too many at once): what classifying it took is
    /// given back. Its connection is closed unanswered.
    pub fn unserved(&self, kind: Kind) {
        match kind {
            Kind::Registrar { enrollment, .. } => self.release_registrar(enrollment.uid),
            Kind::Application(registration) => give_back(&self.open_applications, registration.uid),
            Kind::Refused => {}
        }
    }

    fn release_registrar(&self, uid: u32) {
        give_back(&self.open_registrars, uid);
    }

    fn drop_registrar(&self, registrar: u64) {
        let mut registrations = self.registrations.lock().unwrap_or_else(|p| p.into_inner());
        registrations.retain(|registration| {
            let keep = registration.registrar != registrar;
            if !keep {
                let _ = rustix::io::write(&registration.ended, &1u64.to_ne_bytes());
            }
            keep
        });
    }

    fn drop_registration(&self, registration: &Arc<Registration>) {
        let mut registrations = self.registrations.lock().unwrap_or_else(|p| p.into_inner());
        registrations.retain(|held| !Arc::ptr_eq(held, registration));
        let _ = rustix::io::write(&registration.ended, &1u64.to_ne_bytes());
    }

    fn serve_registrar(
        &self,
        connection: &Connection,
        id: u64,
        enrollment: &Enrollment,
        registrar_pid: i32,
        log: &mut impl FnMut(&Event),
    ) -> io::Result<()> {
        connection.set_receive_timeout(None)?;
        loop {
            // The connection, and each of its registrations' processes: one that exits is
            // forgotten.
            let mine: Vec<Arc<Registration>> = self
                .registrations
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .iter()
                .filter(|registration| registration.registrar == id)
                .cloned()
                .collect();
            let mut fds = vec![PollFd::new(connection, PollFlags::IN)];
            fds.extend(
                mine.iter()
                    .map(|registration| PollFd::new(&registration.pidfd, PollFlags::IN)),
            );
            match poll(&mut fds, None) {
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => return Err(error.into()),
            }
            let message_waiting = !fds[0].revents().is_empty();
            let exited: Vec<bool> = fds[1..].iter().map(|fd| !fd.revents().is_empty()).collect();
            drop(fds);
            for (registration, exited) in mine.iter().zip(exited) {
                if exited {
                    self.drop_registration(registration);
                }
            }
            if !message_waiting {
                continue;
            }
            let Some((request, fds)) = connection.receive_with::<Request>()? else {
                return Ok(());
            };
            let answer = match request {
                Some(Request::Register {
                    desktop_entry_id,
                    system,
                }) => {
                    let answer = self.register(
                        id,
                        enrollment.uid,
                        registrar_pid,
                        &desktop_entry_id,
                        system,
                        fds,
                    );
                    log(&match &answer {
                        Ok(pid) => Event::Registered {
                            id: desktop_entry_id,
                            pid: *pid,
                        },
                        Err(reason) => Event::RegistrationRefused {
                            id: desktop_entry_id,
                            reason: *reason,
                        },
                    });
                    match answer {
                        Ok(_) => Answer::Registered,
                        Err(reason) => Answer::refused(reason),
                    }
                }
                Some(Request::Check { password, local }) => {
                    let unlocking = Secret::new(password.expose());
                    let checked = self
                        .platform
                        .check(enrollment.user.as_str(), password, local);
                    log(&Event::Checked(checked));
                    connection.send(&Answer::Checked(checked))?;
                    // Accepted, the password opens the account's keychain (ADR 0050 decision 2),
                    // after the lock has its answer: the keybag's Argon2id never delays it.
                    if checked.result == platform::Verdict::Accepted {
                        if let Some(keychains) = self.platform.keychains() {
                            keychains.unlock(enrollment.uid, enrollment.user.as_str(), &unlocking);
                        }
                    }
                    continue;
                }
                Some(request) if keychain::is_registrar_request(&request) => {
                    match self.platform.keychains() {
                        Some(keychains) => {
                            keychains.registrar(enrollment.uid, enrollment.user.as_str(), request)
                        }
                        None => Answer::refused(Reason::Locked),
                    }
                }
                Some(_) => Answer::refused(Reason::NotPermitted),
                None => Answer::refused(Reason::InvalidRequest),
            };
            connection.send(&answer)?;
        }
    }

    /// Registers the process `fds` hands over, if it is the registrar's own child, of its user.
    fn register(
        &self,
        registrar: u64,
        uid: u32,
        registrar_pid: i32,
        desktop_entry_id: &str,
        system: bool,
        mut fds: Vec<OwnedFd>,
    ) -> Result<i32, Reason> {
        if fds.len() != 1 || !valid_entry_id(desktop_entry_id) {
            return Err(Reason::InvalidRequest);
        }
        let pidfd = fds.remove(0);
        let (pid, status) =
            pidfd::while_alive(&pidfd, pidfd::status).ok_or(Reason::InvalidRequest)?;
        if status.parent != registrar_pid || status.uid != uid {
            return Err(Reason::NotPermitted);
        }
        let mut registrations = self.registrations.lock().unwrap_or_else(|p| p.into_inner());
        // A pid registered again (its earlier process gone) is the new one's.
        registrations.retain(|held| {
            let keep = !(held.pid == pid && held.uid == uid);
            if !keep {
                let _ = rustix::io::write(&held.ended, &1u64.to_ne_bytes());
            }
            keep
        });
        if registrations
            .iter()
            .filter(|held| held.registrar == registrar)
            .count()
            >= MAX_REGISTRATIONS
        {
            return Err(Reason::TooMany);
        }
        let ended = eventfd().map_err(|_| Reason::Failed)?;
        registrations.push(Arc::new(Registration {
            desktop_entry_id: desktop_entry_id.to_owned(),
            system,
            uid,
            pid,
            pidfd,
            registrar,
            ended,
            subscriptions: Mutex::new(0),
        }));
        Ok(pid)
    }

    fn live(&self, registration: &Arc<Registration>) -> bool {
        let held = self
            .registrations
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .any(|held| Arc::ptr_eq(held, registration));
        held && registration.alive()
    }

    fn serve_application(
        &self,
        connection: &Connection,
        registration: &Arc<Registration>,
        log: &mut impl FnMut(&Event),
    ) -> io::Result<()> {
        connection.set_receive_timeout(Some(IDLE))?;
        let uid = registration.uid;
        loop {
            let Some(request) = connection.receive::<Request>()? else {
                return Ok(());
            };
            if !self.live(registration) {
                return connection.send(&Answer::refused(Reason::NotPermitted));
            }
            let answer = match request {
                Some(Request::Snapshot) => Answer::Snapshot(self.platform.snapshot(uid)),
                Some(Request::Token { reason }) => {
                    let attestation =
                        platform::attestation(&registration.desktop_entry_id, registration.system);
                    Answer::Token(self.platform.token(uid, &attestation, reason))
                }
                Some(Request::Subscribe) => return self.subscribe(connection, registration),
                Some(Request::Remove {}) => {
                    if !(registration.system
                        && registration.desktop_entry_id == platform::OWN_APPLICATIONS[0])
                    {
                        Answer::refused(Reason::NotPermitted)
                    } else {
                        match self.platform.remove(uid) {
                            Ok(enrollment) => {
                                log(&Event::Removed);
                                let sent = connection.send(&Answer::Removed);
                                self.platform.finish_removal(&enrollment);
                                return sent;
                            }
                            Err(error) => {
                                eprintln!("oxyos-accountd: uid {uid}: not removed: {error:?}");
                                Answer::refused(Reason::Failed)
                            }
                        }
                    }
                }
                Some(Request::Register { .. } | Request::Check { .. }) => {
                    Answer::refused(Reason::NotPermitted)
                }
                // The keychain is Settings' alone to manage (ADR 0050 decision 5), and AutoFill
                // the registrar's: any other application never learns whether it holds anything.
                Some(request) if keychain::is_keychain_request(&request) => {
                    let settings = registration.system
                        && registration.desktop_entry_id == platform::OWN_APPLICATIONS[0];
                    match (
                        settings && !keychain::is_registrar_request(&request),
                        self.platform.keychains(),
                        self.platform.enrollment_of(uid),
                    ) {
                        (true, Some(keychains), Ok(Some(enrollment))) => {
                            keychains.settings(uid, enrollment.user.as_str(), request)
                        }
                        (true, None, _) => Answer::refused(Reason::Locked),
                        (true, _, _) => Answer::refused(Reason::Failed),
                        (false, _, _) => Answer::refused(Reason::NotPermitted),
                    }
                }
                Some(_) => Answer::refused(Reason::NotPermitted),
                None => Answer::refused(Reason::InvalidRequest),
            };
            connection.send(&answer)?;
        }
    }

    /// The snapshot now, and again on every change, until the client closes or its registration
    /// ends. A message it sends is read and answered `invalidRequest`, and ends it.
    fn subscribe(
        &self,
        connection: &Connection,
        registration: &Arc<Registration>,
    ) -> io::Result<()> {
        {
            let mut subscriptions = registration
                .subscriptions
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if *subscriptions >= MAX_SUBSCRIPTIONS {
                return connection.send(&Answer::refused(Reason::TooMany));
            }
            *subscriptions += 1;
        }
        let subscribed = self.subscribed(connection, registration);
        *registration
            .subscriptions
            .lock()
            .unwrap_or_else(|p| p.into_inner()) -= 1;
        subscribed
    }

    fn subscribed(
        &self,
        connection: &Connection,
        registration: &Arc<Registration>,
    ) -> io::Result<()> {
        let uid = registration.uid;
        let waker = Arc::new(eventfd()?);
        self.platform.watch(uid, &waker);
        let mut sent = self.platform.snapshot(uid);
        connection.send(&Answer::Snapshot(sent.clone()))?;
        connection.set_receive_timeout(None)?;
        loop {
            let mut fds = [
                PollFd::new(connection, PollFlags::IN),
                PollFd::new(&*waker, PollFlags::IN),
                PollFd::new(&registration.ended, PollFlags::IN),
            ];
            match poll(&mut fds, None) {
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => return Err(error.into()),
            }
            let (message, changed, ended) = (
                !fds[0].revents().is_empty(),
                !fds[1].revents().is_empty(),
                !fds[2].revents().is_empty(),
            );
            if message {
                return match connection.receive::<serde::de::IgnoredAny>()? {
                    None => Ok(()),
                    Some(_) => connection.send(&Answer::refused(Reason::InvalidRequest)),
                };
            }
            if changed {
                drain(&waker);
                let now = self.platform.snapshot(uid);
                if now != sent {
                    connection.send(&Answer::Snapshot(now.clone()))?;
                    sent = now;
                }
            }
            if ended {
                // Its process or its session is gone: nothing more is told.
                return Ok(());
            }
        }
    }
}

/// A desktop entry id as Oxy's registry takes one: `so.oxy.dropsea`.
pub fn valid_entry_id(id: &str) -> bool {
    let mut parts = id.split('.');
    let first = parts.next().unwrap_or("");
    let rest: Vec<&str> = parts.collect();
    id.len() <= 255
        && !first.is_empty()
        && first
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && !rest.is_empty()
        && rest.iter().all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

/// Answers a peer that may do nothing here: its first message read, then refused.
fn refuse(connection: &Connection) -> io::Result<()> {
    crate::ipc::refuse(connection, crate::ipc::Reason::NotPermitted)
}

/// How long the session's clients wait for an answer to a registration.
pub const REGISTER_WAIT: Duration = Duration::from_secs(5);

/// The session's connection as a registrar: one for its life.
pub struct Registrar {
    connection: Connection,
}

impl Registrar {
    pub fn connect(socket: &Path) -> io::Result<Self> {
        let connection = Connection::connect(socket)?;
        connection.set_timeout(REGISTER_WAIT)?;
        Ok(Self { connection })
    }

    /// Registers the process `pidfd` is, launched as `desktop_entry_id`.
    pub fn register(
        &self,
        desktop_entry_id: &str,
        system: bool,
        pidfd: BorrowedFd<'_>,
    ) -> io::Result<Result<(), Reason>> {
        self.connection.send_with(
            &Request::Register {
                desktop_entry_id: desktop_entry_id.to_owned(),
                system,
            },
            pidfd,
        )?;
        match self.connection.receive::<Answer>()? {
            Some(Some(Answer::Registered)) => Ok(Ok(())),
            Some(Some(Answer::Refused { reason })) => Ok(Err(reason)),
            Some(_) => Err(io::Error::new(io::ErrorKind::InvalidData, "not an answer")),
            None => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the service closed the connection",
            )),
        }
    }
}

impl Registrar {
    /// Locks the session's keychain (the session locked).
    pub fn keychain_lock(&self) -> io::Result<Result<(), Reason>> {
        self.connection.send(&Request::KeychainLock {})?;
        match self.connection.receive::<Answer>()? {
            Some(Some(Answer::KeychainLocked)) => Ok(Ok(())),
            Some(Some(Answer::Refused { reason })) => Ok(Err(reason)),
            Some(_) => Err(io::Error::new(io::ErrorKind::InvalidData, "not an answer")),
            None => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the service closed the connection",
            )),
        }
    }
}

impl AsFd for Registrar {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.connection.as_fd()
    }
}

/// One request to the session socket and its one answer, within `wait`.
pub fn ask(socket: &Path, request: &Request, wait: Duration) -> io::Result<Answer> {
    let connection = Connection::connect(socket)?;
    connection.set_timeout(wait)?;
    connection.send(request)?;
    match connection.receive::<Answer>()? {
        Some(Some(answer)) => Ok(answer),
        Some(None) => Err(io::Error::new(io::ErrorKind::InvalidData, "not an answer")),
        None => Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the service closed the connection",
        )),
    }
}

/// The lock's online check of the session's own account, after PAM said `local`. Fails if the
/// service does not answer or refuses: the lock then keeps PAM's answer.
pub fn check(socket: &Path, password: Secret, local: Local) -> io::Result<Checked> {
    match ask(
        socket,
        &Request::Check { password, local },
        crate::ipc::CHECK_WAIT,
    )? {
        Answer::Checked(checked) => Ok(checked),
        Answer::Refused { reason } => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("refused: {reason:?}"),
        )),
        _ => Err(io::Error::new(io::ErrorKind::InvalidData, "not an answer")),
    }
}

/// The session's registrar, on a thread of its own: a launch hands it the process and goes on,
/// never waiting for the service. It keeps one connection for the session's life. Each process
/// it was given is registered on it while the process lives; if the service goes (restarted), it
/// connects again (after 1, 2, 4, 8 and 16 s, then at the next launch) and registers again every
/// process still running. Without the service nothing is registered, and applications simply get
/// no token.
pub struct RegistrarThread {
    launches: std::sync::mpsc::Sender<Job>,
    wake: std::os::unix::net::UnixStream,
}

/// What the registrar's thread is given to do.
enum Job {
    Register(String, bool, OwnedFd),
    /// The session locked: its keychain locks too (ADR 0050 decision 2).
    KeychainLock,
}

/// The waits before connecting again to a service that went.
const RECONNECT: [Duration; 5] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(16),
];

impl RegistrarThread {
    /// The registrar of the service at `socket`; `log` says what it could not do.
    pub fn spawn(socket: PathBuf, log: impl Fn(&str) + Send + 'static) -> io::Result<Self> {
        let (launches, received) = std::sync::mpsc::channel();
        let (wake, woken) = std::os::unix::net::UnixStream::pair()?;
        woken.set_nonblocking(true)?;
        std::thread::Builder::new()
            .name("oxyos-registrar".into())
            .spawn(move || registrar_thread(&socket, &received, &woken, &log))?;
        Ok(Self { launches, wake })
    }

    /// Registers the process `pidfd` is, launched as `desktop_entry_id` (`system`: a system
    /// application of the image's policy).
    pub fn register(&self, desktop_entry_id: &str, system: bool, pidfd: OwnedFd) {
        self.send(Job::Register(desktop_entry_id.to_owned(), system, pidfd));
    }

    /// Tells the service the session locked, so the account's keychain locks with it. Without
    /// the service there is nothing unlocked to lock.
    pub fn keychain_lock(&self) {
        self.send(Job::KeychainLock);
    }

    fn send(&self, job: Job) {
        if self.launches.send(job).is_ok() {
            use std::io::Write;
            let _ = (&self.wake).write(&[1]);
        }
    }
}

fn registrar_thread(
    socket: &Path,
    launches: &std::sync::mpsc::Receiver<Job>,
    woken: &std::os::unix::net::UnixStream,
    log: &dyn Fn(&str),
) {
    use std::io::Read;
    use std::time::Instant;
    let mut live: Vec<(String, bool, OwnedFd)> = Vec::new();
    let mut registrar: Option<Registrar> = None;
    // The next attempt to connect again, and how many were made since the service went.
    let mut retry: Option<(Instant, usize)> = None;
    let mut said_unreachable = false;
    let connect = |live: &[(String, bool, OwnedFd)], said: &mut bool| -> Option<Registrar> {
        let registrar = match Registrar::connect(socket) {
            Ok(registrar) => registrar,
            Err(error) => {
                if !std::mem::replace(said, true) {
                    log(&format!(
                        "the accounts' service at {}: {error}; launches are not registered",
                        socket.display()
                    ));
                }
                return None;
            }
        };
        *said = false;
        for (id, system, pidfd) in live {
            if !register_one(&registrar, id, *system, pidfd, log) {
                return None;
            }
        }
        Some(registrar)
    };
    loop {
        let timeout = retry.map(|(at, _)| {
            let wait = at.saturating_duration_since(Instant::now());
            rustix::time::Timespec {
                tv_sec: wait.as_secs() as i64,
                tv_nsec: i64::from(wait.subsec_nanos()),
            }
        });
        let mut fds = vec![PollFd::new(woken, PollFlags::IN)];
        if let Some(registrar) = &registrar {
            fds.push(PollFd::new(registrar, PollFlags::IN));
        }
        let first_pidfd = fds.len();
        fds.extend(
            live.iter()
                .map(|(_, _, pidfd)| PollFd::new(pidfd, PollFlags::IN)),
        );
        if let Err(error) = poll(&mut fds, timeout.as_ref()) {
            if error != rustix::io::Errno::INTR {
                log(&format!("waiting: {error}"));
                return;
            }
            continue;
        }
        let woke = !fds[0].revents().is_empty();
        let lost = registrar.is_some() && !fds[1].revents().is_empty();
        let exited: Vec<bool> = fds[first_pidfd..]
            .iter()
            .map(|fd| !fd.revents().is_empty())
            .collect();
        drop(fds);
        // Processes that exited are forgotten (the service saw them go too).
        let mut exited = exited.into_iter();
        live.retain(|_| !exited.next().unwrap_or(false));
        if lost {
            // The service closed the connection (it stopped): its registrations went with it.
            registrar = None;
            retry = Some((Instant::now() + RECONNECT[0], 0));
        }
        if woke {
            let mut bytes = [0u8; 64];
            // The session let go of its registrar: it is ending.
            if let Ok(0) = (&*woken).read(&mut bytes) {
                return;
            }
            while let Ok(job) = launches.try_recv() {
                let (id, system, pidfd) = match job {
                    Job::Register(id, system, pidfd) => (id, system, pidfd),
                    Job::KeychainLock => {
                        if let Some(connected) = &registrar {
                            if !lock_keychain(connected, log) {
                                registrar = None;
                                retry = Some((Instant::now() + RECONNECT[0], 0));
                            }
                        }
                        continue;
                    }
                };
                if let Some(connected) = &registrar {
                    if !register_one(connected, &id, system, &pidfd, log) {
                        registrar = None;
                        retry = Some((Instant::now() + RECONNECT[0], 0));
                    }
                }
                live.push((id, system, pidfd));
            }
            // Not connected and not waiting to: a launch is the time to try again.
            if registrar.is_none() && retry.is_none() {
                registrar = connect(&live, &mut said_unreachable);
            }
        }
        if let Some((at, made)) = retry {
            if Instant::now() >= at {
                registrar = connect(&live, &mut said_unreachable);
                retry = match (&registrar, RECONNECT.get(made + 1)) {
                    (Some(_), _) | (None, None) => None,
                    (None, Some(wait)) => Some((Instant::now() + *wait, made + 1)),
                };
            }
        }
    }
}

/// Locks the session's keychain; false if the connection failed (the service went, and with it
/// whatever it had unlocked).
fn lock_keychain(registrar: &Registrar, log: &dyn Fn(&str)) -> bool {
    match registrar.keychain_lock() {
        Ok(Ok(())) => true,
        Ok(Err(reason)) => {
            log(&format!("the keychain was not locked: {reason:?}"));
            true
        }
        Err(error) => {
            log(&format!("locking the keychain: {error}"));
            false
        }
    }
}

/// Registers one process; false if the connection failed (the service went).
fn register_one(
    registrar: &Registrar,
    id: &str,
    system: bool,
    pidfd: &OwnedFd,
    log: &dyn Fn(&str),
) -> bool {
    match registrar.register(id, system, pidfd.as_fd()) {
        Ok(Ok(())) => true,
        Ok(Err(reason)) => {
            log(&format!("{id} was not registered: {reason:?}"));
            true
        }
        Err(error) => {
            log(&format!("registering {id}: {error}"));
            false
        }
    }
}
