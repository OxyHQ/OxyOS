//! C ABI over `oxyos-accountd`'s two sockets (docs/adr/0020-oxy-account-login.mdx), for the Bloom
//! host:
//!
//! - **The machine's Oxy session, for an application** (decision 6): the session socket,
//!   `/run/oxyos/accountd/session` (`OXYOS_ACCOUNTD_SESSION_SOCKET` for scenarios). The service
//!   knows the asking process by its launch, never by what it says: the application asks for the
//!   session's snapshot, follows it, and asks for a short token for itself. Settings may ask the
//!   service to remove its own account from the machine.
//! - **Setting an account up, for the login screen** (decision 4): the control socket's `platform`
//!   (Oxy's API and OxyOS's public client id, which the SDK's sign-in runs with) and the `enroll`
//!   conversation, a second factor answered on the same connection.
//!
//! Both are `SOCK_SEQPACKET` with one JSON message per packet, in `oxyos-account`'s own types (`ipc`,
//! `session`, `platform`): one definition of the protocol. What the service answers is handed on
//! rebuilt, never passed through: a snapshot that does not hold together, or a token without its
//! expiry, is no answer. Secrets (a password, a device
//! secret, a second factor's code, an access token) are held in buffers overwritten when dropped
//! and never printed: not in an error, not in `oxyos_last_error`.

use super::fail;
use oxyos_account::ipc::{self, Connection};
pub use oxyos_account::platform::{Snapshot, SnapshotAccount, Status, Token, TokenReason};
use oxyos_account::session;
use oxyos_account::Secret;
use serde::{Deserialize, Serialize};
use std::ffi::{c_char, c_int, CStr};
use std::os::fd::{AsFd, AsRawFd};
use std::path::{Path, PathBuf};
use std::time::Duration;
use zeroize::Zeroizing;

/// How long a snapshot may take: the service answers it from memory.
const SNAPSHOT_WAIT: Duration = Duration::from_secs(5);
/// How long a token may take: a mint asks Oxy online.
const TOKEN_WAIT: Duration = Duration::from_secs(60);
/// How long removing an account may take: a revocation online, the sessions ended, the user gone.
const REMOVE_WAIT: Duration = Duration::from_secs(120);
/// How long `platform` may take.
const PLATFORM_WAIT: Duration = Duration::from_secs(5);
/// How long a setup's turn may take: a sign-in online and the user made.
const ENROLL_WAIT: Duration = Duration::from_secs(180);

/// The session socket: `OXYOS_ACCOUNTD_SESSION_SOCKET`, else the system's.
fn session_socket() -> PathBuf {
    std::env::var_os("OXYOS_ACCOUNTD_SESSION_SOCKET")
        .map_or_else(|| session::SOCKET.into(), PathBuf::from)
}

/// The control socket: `OXYOS_ACCOUNTD_SOCKET`, else the system's.
fn control_socket() -> PathBuf {
    std::env::var_os("OXYOS_ACCOUNTD_SOCKET").map_or_else(|| ipc::SOCKET.into(), PathBuf::from)
}

// ---------------------------------------------------------------------------------------------
// The session socket (an application's), in `oxyos-account`'s own types

/// Whether a snapshot holds together: signed in names an account, anything else none.
fn coherent(snapshot: &Snapshot) -> bool {
    match (&snapshot.status, &snapshot.account) {
        (Status::SignedIn, Some(account)) => !account.id.is_empty() && !account.handle.is_empty(),
        (Status::SignedIn, None) => false,
        (_, account) => account.is_none(),
    }
}

/// The SDK's reason by its name.
pub fn token_reason(text: &str) -> Option<TokenReason> {
    match text {
        "boot" => Some(TokenReason::Boot),
        "preflight" => Some(TokenReason::Preflight),
        "response-401" => Some(TokenReason::Response401),
        _ => None,
    }
}

/// A refusal's reason by its name on the wire (`notPermitted`).
fn reason_name(reason: &impl Serialize) -> String {
    serde_json::to_value(reason)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "refused".into())
}

/// Why the service could not be asked or refused: never a secret.
#[derive(Debug, PartialEq, Eq)]
pub enum SessionError {
    /// The service refused, with its reason (`notPermitted` for a process that is not a launched
    /// application's).
    Refused(String),
    /// It could not be reached, or answered something else.
    Failed(String),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::Refused(reason) => write!(f, "the accounts' service refused: {reason}"),
            SessionError::Failed(why) => f.write_str(why),
        }
    }
}

fn failed(socket: &Path, error: impl std::fmt::Display) -> SessionError {
    SessionError::Failed(format!(
        "the accounts' service at {}: {error}",
        socket.display()
    ))
}

/// One request on a connection of its own, and its one answer.
fn ask(
    socket: &Path,
    request: &session::Request,
    wait: Duration,
) -> Result<session::Answer, SessionError> {
    let connection = Connection::connect(socket).map_err(|error| failed(socket, error))?;
    connection
        .set_timeout(wait)
        .map_err(|error| failed(socket, error))?;
    connection
        .send(request)
        .map_err(|error| failed(socket, error))?;
    match connection.receive::<session::Answer>() {
        Ok(Some(Some(session::Answer::Refused { reason }))) => {
            Err(SessionError::Refused(reason_name(&reason)))
        }
        Ok(Some(Some(answer))) => Ok(answer),
        Ok(Some(None)) => Err(failed(socket, "an answer that is not one")),
        Ok(None) => Err(failed(socket, "it closed the connection")),
        Err(error) => Err(failed(socket, error)),
    }
}

/// The session's snapshot, asked once.
pub fn snapshot_at(socket: &Path) -> Result<Snapshot, SessionError> {
    match ask(socket, &session::Request::Snapshot, SNAPSHOT_WAIT)? {
        session::Answer::Snapshot(snapshot) if coherent(&snapshot) => Ok(snapshot),
        _ => Err(failed(socket, "not a snapshot")),
    }
}

/// A token for the asking application, or none (offline, signed out, or not given).
pub fn token_at(socket: &Path, reason: TokenReason) -> Result<Option<Token>, SessionError> {
    match ask(socket, &session::Request::Token { reason }, TOKEN_WAIT)? {
        session::Answer::Token(Some(token))
            if !token.access_token.is_empty() && !token.expires_at.is_empty() =>
        {
            Ok(Some(token))
        }
        session::Answer::Token(None) => Ok(None),
        _ => Err(failed(socket, "not a token")),
    }
}

/// Removes the asking application's account from the machine (Settings alone may).
pub fn remove_at(socket: &Path) -> Result<(), SessionError> {
    match ask(socket, &session::Request::Remove {}, REMOVE_WAIT)? {
        session::Answer::Removed => Ok(()),
        _ => Err(failed(socket, "not an answer to remove")),
    }
}

/// The snapshots as they change, on one connection the service keeps: the first at once.
pub struct Subscription {
    connection: Connection,
    socket: PathBuf,
    /// A snapshot read that did not fit the caller's buffer, handed over first.
    pending: Option<String>,
}

impl Subscription {
    pub fn connect(socket: &Path) -> Result<Self, SessionError> {
        let connection = Connection::connect(socket).map_err(|error| failed(socket, error))?;
        connection
            .send(&session::Request::Subscribe)
            .map_err(|error| failed(socket, error))?;
        Ok(Self {
            connection,
            socket: socket.to_owned(),
            pending: None,
        })
    }

    /// The next snapshot as JSON if one arrived (`Ok(None)`: none yet), or why it ended.
    pub fn arrived(&mut self) -> Result<Option<String>, SessionError> {
        if let Some(json) = self.pending.take() {
            return Ok(Some(json));
        }
        let mut readable = [rustix::event::PollFd::new(
            &self.connection,
            rustix::event::PollFlags::IN,
        )];
        let ready = rustix::event::poll(&mut readable, Some(&rustix::event::Timespec::default()))
            .map_err(|error| failed(&self.socket, error))?;
        if ready == 0 {
            return Ok(None);
        }
        match self.connection.receive::<session::Answer>() {
            Ok(Some(Some(session::Answer::Snapshot(snapshot)))) if coherent(&snapshot) => {
                serde_json::to_string(&snapshot)
                    .map(Some)
                    .map_err(|error| failed(&self.socket, error))
            }
            Ok(Some(Some(session::Answer::Refused { reason }))) => {
                Err(SessionError::Refused(reason_name(&reason)))
            }
            Ok(Some(_)) => Err(failed(&self.socket, "not a snapshot")),
            Ok(None) => Err(failed(&self.socket, "it closed the subscription")),
            Err(error) => Err(failed(&self.socket, error)),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The control socket (the login screen's)

/// What the login screen's setup runs the SDK's sign-in with: Oxy's API and OxyOS's public client
/// id (C2's `platform`). Neither is secret.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Platform {
    pub api: String,
    pub client_id: String,
}

/// The platform the service sets accounts up for: one without a client id is not configured.
pub fn platform_at(socket: &Path) -> Result<Platform, SessionError> {
    let connection = Connection::connect(socket).map_err(|error| failed(socket, error))?;
    connection
        .set_timeout(PLATFORM_WAIT)
        .map_err(|error| failed(socket, error))?;
    connection
        .send(&ipc::Request::Platform)
        .map_err(|error| failed(socket, error))?;
    match connection.receive::<ipc::Answer>() {
        Ok(Some(Some(ipc::Answer::Platform(told)))) => match told.client_id {
            _ if told.api.is_empty() => Err(failed(socket, "a platform without its API")),
            Some(client_id) if !client_id.is_empty() => Ok(Platform {
                api: told.api,
                client_id,
            }),
            _ => Err(SessionError::Refused("notConfigured".into())),
        },
        Ok(Some(Some(ipc::Answer::Refused { reason, .. }))) => {
            Err(SessionError::Refused(reason_name(&reason)))
        }
        Ok(Some(_)) => Err(failed(socket, "not a platform")),
        Ok(None) => Err(failed(socket, "it closed the connection")),
        Err(error) => Err(failed(socket, error)),
    }
}

/// A setup's answer as the login screen reads it (none is secret): whether it goes on.
fn enroll_json(answer: &ipc::Answer) -> Option<(String, bool)> {
    let (json, continues) = match answer {
        ipc::Answer::SecondFactorRequired => (serde_json::json!("secondFactorRequired"), true),
        ipc::Answer::Enrolled {
            account_id, user, ..
        } => (
            serde_json::json!({"enrolled": {"accountId": account_id, "user": user}}),
            false,
        ),
        ipc::Answer::Refused {
            reason,
            retry_after_seconds,
        } => (
            serde_json::json!({"refused": {"reason": reason, "retryAfterSeconds": retry_after_seconds}}),
            *reason == ipc::Reason::SecondFactorInvalid,
        ),
        _ => return None,
    };
    Some((json.to_string(), continues))
}

/// One setup: an `enroll`, then second factors while the service asks for one.
pub struct Enrollment {
    connection: Option<Connection>,
    socket: PathBuf,
    /// Whether `enroll` was sent.
    started: bool,
    /// The last answer, as JSON, for a caller whose buffer was too small.
    answer: String,
}

impl Enrollment {
    pub fn connect(socket: &Path) -> Result<Self, SessionError> {
        let connection = Connection::connect(socket).map_err(|error| failed(socket, error))?;
        connection
            .set_timeout(ENROLL_WAIT)
            .map_err(|error| failed(socket, error))?;
        Ok(Self {
            connection: Some(connection),
            socket: socket.to_owned(),
            started: false,
            answer: String::new(),
        })
    }

    fn turn(&mut self, request: &ipc::Request) -> Result<&str, SessionError> {
        let connection = self
            .connection
            .as_ref()
            .ok_or_else(|| SessionError::Failed("the setup has ended".into()))?;
        let result = connection
            .send(request)
            .map_err(|error| failed(&self.socket, error))
            .and_then(|()| match connection.receive::<ipc::Answer>() {
                Ok(Some(Some(answer))) => enroll_json(&answer)
                    .ok_or_else(|| failed(&self.socket, "not an answer to a setup")),
                Ok(Some(None)) => Err(failed(&self.socket, "an answer that is not one")),
                Ok(None) => Err(failed(&self.socket, "it closed the connection")),
                Err(error) => Err(failed(&self.socket, error)),
            });
        match result {
            Ok((json, continues)) => {
                if !continues {
                    self.connection = None;
                }
                self.answer = json;
                Ok(&self.answer)
            }
            Err(error) => {
                self.connection = None;
                Err(error)
            }
        }
    }

    /// Sends the setup: the device credential the SDK's sign-in obtained, the account it is for and
    /// the account's password. The service's answer as JSON.
    pub fn enroll(
        &mut self,
        device_id: &str,
        device_secret: &Secret,
        account_id: &str,
        password: &Secret,
    ) -> Result<&str, SessionError> {
        if self.started {
            return Err(SessionError::Failed("the setup was sent already".into()));
        }
        self.started = true;
        self.turn(&ipc::Request::Enroll {
            device_id: device_id.to_owned(),
            device_secret: Secret::new(device_secret.expose()),
            account_id: account_id.to_owned(),
            password: Secret::new(password.expose()),
        })
    }

    /// Answers the second factor the service asked for.
    pub fn second_factor(&mut self, code: &Secret) -> Result<&str, SessionError> {
        if !self.started {
            return Err(SessionError::Failed("no setup was sent".into()));
        }
        self.turn(&ipc::Request::SecondFactor {
            code: Secret::new(code.expose()),
        })
    }
}

// ---------------------------------------------------------------------------------------------
// The C ABI

/// Writes `text` into `out` (`capacity` bytes, NUL-terminated when it fits); its length.
///
/// # Safety
/// `out` points to `capacity` writable bytes, or is null with `capacity` 0.
#[allow(unsafe_code)]
unsafe fn write_out(text: &[u8], out: *mut c_char, capacity: usize) -> i64 {
    if text.len() < capacity {
        // SAFETY: `out` has `capacity` bytes, more than the text and its NUL.
        unsafe {
            std::ptr::copy_nonoverlapping(text.as_ptr(), out.cast::<u8>(), text.len());
            *out.add(text.len()) = 0;
        }
    }
    text.len() as i64
}

fn session_fail(error: SessionError) -> i64 {
    i64::from(fail(error))
}

/// A NUL-terminated string the caller passes, as a secret: copied into a buffer overwritten when
/// dropped. None if null or not UTF-8.
///
/// # Safety
/// `text` is null or a NUL-terminated string valid for the call.
#[allow(unsafe_code)]
unsafe fn secret_arg(text: *const c_char) -> Option<Secret> {
    if text.is_null() {
        return None;
    }
    // SAFETY: the caller's contract.
    let bytes = unsafe { CStr::from_ptr(text) }.to_bytes();
    std::str::from_utf8(bytes).ok().map(Secret::new)
}

/// # Safety
/// As `secret_arg`.
#[allow(unsafe_code)]
unsafe fn text_arg<'a>(text: *const c_char) -> Option<&'a str> {
    if text.is_null() {
        return None;
    }
    // SAFETY: the caller's contract.
    std::str::from_utf8(unsafe { CStr::from_ptr(text) }.to_bytes()).ok()
}

/// The session's snapshot as JSON (`{"status", "revision", "account", "online"}`), asked once,
/// into `out`. Its length (larger than `capacity`: call again with more room), or OXYOS_ERROR
/// (`oxyos_last_error`: unreachable, or refused, `notPermitted` to a process the session did not
/// launch).
///
/// # Safety
/// `out` points to `capacity` writable bytes, or is null with `capacity` 0.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_account_session_snapshot(out: *mut c_char, capacity: usize) -> i64 {
    if out.is_null() && capacity != 0 {
        return i64::from(fail("null buffer"));
    }
    match snapshot_at(&session_socket()).and_then(|snapshot| {
        serde_json::to_string(&snapshot).map_err(|error| SessionError::Failed(error.to_string()))
    }) {
        // SAFETY: the caller's contract.
        Ok(json) => unsafe { write_out(json.as_bytes(), out, capacity) },
        Err(error) => session_fail(error),
    }
}

/// A token for the calling application (`reason` "boot", "preflight" or "response-401", which
/// mints afresh), as JSON `{"accessToken", "expiresAt"}`, or `null` when there is none (offline,
/// signed out). Blocks while the service mints online: call it off the interface's thread. The
/// token is secret: this library's copies are overwritten; the caller overwrites `out` once it
/// has handed the token on. Its length (larger than `capacity`: nothing written, and the token
/// is lost: give OXYOS_ACCOUNT_MESSAGE_MAX bytes), or OXYOS_ERROR (`oxyos_last_error`).
///
/// # Safety
/// `reason` is a NUL-terminated string valid for the call; `out` points to `capacity` writable
/// bytes, or is null with `capacity` 0.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_account_session_token(
    reason: *const c_char,
    out: *mut c_char,
    capacity: usize,
) -> i64 {
    if out.is_null() && capacity != 0 {
        return i64::from(fail("null buffer"));
    }
    // SAFETY: the caller's contract.
    let Some(reason) = (unsafe { text_arg(reason) }).and_then(token_reason) else {
        return i64::from(fail("a token's reason: boot, preflight or response-401"));
    };
    match token_at(&session_socket(), reason) {
        Ok(token) => {
            let mut json = Zeroizing::new(Vec::with_capacity(4096));
            if let Err(error) = serde_json::to_writer(&mut *json, &token) {
                return i64::from(fail(error));
            }
            // SAFETY: the caller's contract.
            unsafe { write_out(&json, out, capacity) }
        }
        Err(error) => session_fail(error),
    }
}

/// Removes the calling application's account from this computer (Settings alone may: the service
/// refuses anyone else): its sessions end, its credential, verifier, user and home are forgotten.
/// Blocks while it does. 0, or OXYOS_ERROR (`oxyos_last_error`).
#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn oxyos_account_session_remove() -> c_int {
    match remove_at(&session_socket()) {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

/// The session's snapshots as they change (see oxyos_account_session_subscription_next).
pub struct OxyOSAccountSubscription(Subscription);

/// Follows the session's snapshot: the service sends it at once and again at every change. NULL
/// when it could not be asked (`oxyos_last_error`).
#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn oxyos_account_session_subscribe() -> *mut OxyOSAccountSubscription {
    match Subscription::connect(&session_socket()) {
        Ok(subscription) => Box::into_raw(Box::new(OxyOSAccountSubscription(subscription))),
        Err(error) => {
            fail(error);
            std::ptr::null_mut()
        }
    }
}

/// The descriptor that is readable when a snapshot arrives (poll it, then call `_next`).
///
/// # Safety
/// `subscription` is one `oxyos_account_session_subscribe` returned, not freed.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_account_session_subscription_fd(
    subscription: *const OxyOSAccountSubscription,
) -> c_int {
    if subscription.is_null() {
        return fail("null subscription");
    }
    // SAFETY: the caller's contract.
    unsafe { &*subscription }.0.connection.as_fd().as_raw_fd()
}

/// The next snapshot that arrived, as oxyos_account_session_snapshot writes it, without waiting:
/// its length (larger than `capacity`: nothing written, and the next call gives it again), 0 when
/// none has arrived, or OXYOS_ERROR once the subscription ended (the service went away or refused,
/// `oxyos_last_error`): free it and subscribe again later.
///
/// # Safety
/// `subscription` is one `oxyos_account_session_subscribe` returned, not freed; `out` points to
/// `capacity` writable bytes, or is null with `capacity` 0.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_account_session_subscription_next(
    subscription: *mut OxyOSAccountSubscription,
    out: *mut c_char,
    capacity: usize,
) -> i64 {
    if subscription.is_null() || (out.is_null() && capacity != 0) {
        return i64::from(fail("null subscription or buffer"));
    }
    // SAFETY: the caller's contract.
    let subscription = &mut unsafe { &mut *subscription }.0;
    match subscription.arrived() {
        Ok(None) => 0,
        Ok(Some(json)) => {
            if json.len() >= capacity {
                let length = json.len() as i64;
                subscription.pending = Some(json);
                return length;
            }
            // SAFETY: the caller's contract.
            unsafe { write_out(json.as_bytes(), out, capacity) }
        }
        Err(error) => session_fail(error),
    }
}

/// Ends a subscription.
///
/// # Safety
/// `subscription` is one `oxyos_account_session_subscribe` returned, or null; not used again.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_account_session_subscription_free(
    subscription: *mut OxyOSAccountSubscription,
) {
    if !subscription.is_null() {
        // SAFETY: the caller's contract: it came from Box::into_raw.
        drop(unsafe { Box::from_raw(subscription) });
    }
}

/// What the login screen's setup runs the SDK's sign-in with, from the service's control socket:
/// JSON `{"api", "clientId"}` (neither secret). Its length, or OXYOS_ERROR (`oxyos_last_error`: the service
/// unreachable, or refusing, `notConfigured` without a client id).
///
/// # Safety
/// `out` points to `capacity` writable bytes, or is null with `capacity` 0.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_account_platform(out: *mut c_char, capacity: usize) -> i64 {
    if out.is_null() && capacity != 0 {
        return i64::from(fail("null buffer"));
    }
    match platform_at(&control_socket()).and_then(|platform| {
        serde_json::to_string(&platform).map_err(|error| SessionError::Failed(error.to_string()))
    }) {
        // SAFETY: the caller's contract.
        Ok(json) => unsafe { write_out(json.as_bytes(), out, capacity) },
        Err(error) => session_fail(error),
    }
}

/// A setup on the service's control socket (the login screen's).
pub struct OxyOSAccountEnrollment(Enrollment);

/// Opens a setup's connection. NULL when the service cannot be reached (`oxyos_last_error`).
#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn oxyos_account_enrollment_new() -> *mut OxyOSAccountEnrollment {
    match Enrollment::connect(&control_socket()) {
        Ok(enrollment) => Box::into_raw(Box::new(OxyOSAccountEnrollment(enrollment))),
        Err(error) => {
            fail(error);
            std::ptr::null_mut()
        }
    }
}

/// Writes a setup's answer, or fails.
///
/// # Safety
/// `out` points to `capacity` writable bytes, or is null with `capacity` 0.
#[allow(unsafe_code)]
unsafe fn answered(result: Result<&str, SessionError>, out: *mut c_char, capacity: usize) -> i64 {
    match result {
        // SAFETY: the caller's contract.
        Ok(json) => unsafe { write_out(json.as_bytes(), out, capacity) },
        Err(error) => session_fail(error),
    }
}

/// Sends the setup (`enroll`): the device credential the SDK's sign-in obtained, the account's
/// id and its password, each NUL-terminated UTF-8; the service's answer as JSON:
/// `{"enrolled": {"accountId", "user"}}`, `"secondFactorRequired"` (answer with
/// `_second_factor`) or `{"refused": {"reason", "retryAfterSeconds"}}`. Blocks while the service
/// signs in online. The secrets are copied into buffers this library overwrites; the caller
/// overwrites its own. The answer's length (larger than `capacity`: `_answer` gives it again), or
/// OXYOS_ERROR (`oxyos_last_error`): the setup has ended.
///
/// # Safety
/// `enrollment` is one `oxyos_account_enrollment_new` returned, not freed; the strings are
/// NUL-terminated and valid for the call; `out` points to `capacity` writable bytes, or is null
/// with `capacity` 0.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_account_enrollment_enroll(
    enrollment: *mut OxyOSAccountEnrollment,
    device_id: *const c_char,
    device_secret: *const c_char,
    account_id: *const c_char,
    password: *const c_char,
    out: *mut c_char,
    capacity: usize,
) -> i64 {
    if enrollment.is_null() || (out.is_null() && capacity != 0) {
        return i64::from(fail("null setup or buffer"));
    }
    // SAFETY: the caller's contract.
    let (device_id, device_secret, account_id, password) = unsafe {
        (
            text_arg(device_id),
            secret_arg(device_secret),
            text_arg(account_id),
            secret_arg(password),
        )
    };
    let (Some(device_id), Some(device_secret), Some(account_id), Some(password)) =
        (device_id, device_secret, account_id, password)
    else {
        return i64::from(fail(
            "a setup needs a device id, its secret, an account id and a password",
        ));
    };
    // SAFETY: the caller's contract.
    let enrollment = &mut unsafe { &mut *enrollment }.0;
    let result = enrollment.enroll(device_id, &device_secret, account_id, &password);
    // SAFETY: the caller's contract.
    unsafe { answered(result, out, capacity) }
}

/// Answers the second factor the service asked for (`code`, NUL-terminated UTF-8, secret); the
/// service's answer as `_enroll` gives it (a wrong code: `{"refused": {"reason":
/// "secondFactorInvalid"}}`, and another may be sent).
///
/// # Safety
/// As `oxyos_account_enrollment_enroll`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_account_enrollment_second_factor(
    enrollment: *mut OxyOSAccountEnrollment,
    code: *const c_char,
    out: *mut c_char,
    capacity: usize,
) -> i64 {
    if enrollment.is_null() || (out.is_null() && capacity != 0) {
        return i64::from(fail("null setup or buffer"));
    }
    // SAFETY: the caller's contract.
    let Some(code) = (unsafe { secret_arg(code) }) else {
        return i64::from(fail("a second factor's code"));
    };
    // SAFETY: the caller's contract.
    let enrollment = &mut unsafe { &mut *enrollment }.0;
    let result = enrollment.second_factor(&code);
    // SAFETY: the caller's contract.
    unsafe { answered(result, out, capacity) }
}

/// The setup's last answer again, for a buffer that was too small.
///
/// # Safety
/// As `oxyos_account_enrollment_enroll`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_account_enrollment_answer(
    enrollment: *const OxyOSAccountEnrollment,
    out: *mut c_char,
    capacity: usize,
) -> i64 {
    if enrollment.is_null() || (out.is_null() && capacity != 0) {
        return i64::from(fail("null setup or buffer"));
    }
    // SAFETY: the caller's contract.
    let answer = &unsafe { &*enrollment }.0.answer;
    // SAFETY: the caller's contract.
    unsafe { write_out(answer.as_bytes(), out, capacity) }
}

/// Ends a setup: closing its connection abandons whatever the service had not finished.
///
/// # Safety
/// `enrollment` is one `oxyos_account_enrollment_new` returned, or null; not used again.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_account_enrollment_free(enrollment: *mut OxyOSAccountEnrollment) {
    if !enrollment.is_null() {
        // SAFETY: the caller's contract: it came from Box::into_raw.
        drop(unsafe { Box::from_raw(enrollment) });
    }
}

#[cfg(test)]
mod tests;
