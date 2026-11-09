//! C ABI over the migration service's socket (docs/adr/0059-backups.mdx, decision 7), for the Bloom
//! host run as the login screen: *Migrate from a Backup* on a computer nobody has set up yet.
//!
//! One conversation on `/run/oxyos/migrate/socket` (`OXYOS_MIGRATE_SOCKET` for scenarios),
//! `SOCK_SEQPACKET`, one JSON message per packet (`oxyos-backup`'s `migrate` module is the
//! service's side): the disks, the computers a backup password opens on a disk or a share, then the
//! migration, whose progress arrives as messages before its answer. The service answers root and
//! the login program's user alone, and only while no account is enrolled.
//!
//! The vocabulary is spoken here with types of this module's own, not `oxyos-backup`'s, which
//! would bring the backup service's udev and restic into every process the host runs. What the
//! service answers is read strictly and handed on rebuilt, never passed through. The backup
//! password and a share's password are held in buffers overwritten when dropped, and never printed.

use super::fail;
use oxyos_account::Secret;
use oxyos_install::socket::Connection;
use serde::{Deserialize, Serialize};
use std::ffi::{c_char, c_int, CStr};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long the disks may take: udev's list.
const DISKS_WAIT: Duration = Duration::from_secs(30);
/// How long the backups may take: a disk mounted, or a share reached, and each computer's backups
/// opened with the password (restic's key derivation, a second or so each) and its newest read.
const BACKUPS_WAIT: Duration = Duration::from_secs(10 * 60);
/// How long a migration may say nothing: restic says how far it is once a second while it
/// restores, but making the users, or a slow disk before its first file, may take longer.
const MIGRATE_WAIT: Duration = Duration::from_secs(30 * 60);

/// The service's socket: `OXYOS_MIGRATE_SOCKET`, else the system's.
fn socket() -> PathBuf {
    std::env::var_os("OXYOS_MIGRATE_SOCKET")
        .map_or_else(|| "/run/oxyos/migrate/socket".into(), PathBuf::from)
}

/// A share, as the person named it.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Share {
    host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    share: String,
    user: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    domain: Option<String>,
}

/// Where the backups are.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
enum Source {
    Disk { uuid: String },
    Share { share: Share },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
enum Request<'a> {
    Disks {},
    #[serde(rename_all = "camelCase")]
    Backups {
        source: &'a Source,
        password: &'a Secret,
        #[serde(skip_serializing_if = "Option::is_none")]
        share_password: Option<&'a Secret>,
    },
    Migrate {
        repository: &'a str,
        snapshot: &'a str,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Disk {
    uuid: String,
    label: String,
    fs_type: String,
    size: u64,
}

#[derive(Debug, Deserialize, Serialize)]
struct Backup {
    snapshot: String,
    time: i64,
}

#[derive(Debug, Deserialize, Serialize)]
struct Person {
    user: String,
    name: String,
    #[serde(default)]
    administrator: bool,
}

#[derive(Debug, Deserialize, Serialize)]
struct Computer {
    repository: String,
    name: String,
    backups: Vec<Backup>,
    accounts: Vec<Person>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Listing {
    computers: Vec<Computer>,
    unopened: usize,
}

#[derive(Debug, Deserialize, Serialize)]
struct Migrated {
    user: String,
    name: String,
}

/// What the service answers, read strictly and written again as read.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum Answer {
    Disks(Vec<Disk>),
    Backups(Listing),
    Migrating {
        step: String,
        percent: f64,
    },
    Migrated {
        computer: String,
        accounts: Vec<Migrated>,
    },
    Refused {
        reason: String,
    },
}

/// One conversation with the migration service.
pub struct Migration {
    connection: Option<Connection>,
    socket: PathBuf,
    /// A migration was asked for, and its answer has not arrived yet.
    migrating: bool,
    /// The last answer, as JSON, for a caller whose buffer was too small.
    answer: String,
}

impl Migration {
    pub fn connect(socket: &Path) -> Result<Self, String> {
        let connection = Connection::connect(socket)
            .map_err(|error| format!("the migration service at {}: {error}", socket.display()))?;
        Ok(Self {
            connection: Some(connection),
            socket: socket.to_owned(),
            migrating: false,
            answer: String::new(),
        })
    }

    fn failed(&mut self, error: impl std::fmt::Display) -> String {
        self.connection = None;
        format!(
            "the migration service at {}: {error}",
            self.socket.display()
        )
    }

    fn receive(&mut self, wait: Duration) -> Result<Answer, String> {
        let connection = self
            .connection
            .as_ref()
            .ok_or_else(|| "the conversation has ended".to_owned())?;
        let received = connection
            .set_timeout(wait)
            .and_then(|()| connection.receive::<Answer>());
        match received {
            Ok(Some(Some(answer))) => Ok(answer),
            Ok(Some(None)) => Err(self.failed("an answer that is not one")),
            Ok(None) => Err(self.failed("it closed the connection")),
            Err(error) => Err(self.failed(error)),
        }
    }

    fn send(&mut self, request: &Request<'_>) -> Result<(), String> {
        if self.migrating {
            return Err("a migration is under way".into());
        }
        let connection = self
            .connection
            .as_ref()
            .ok_or_else(|| "the conversation has ended".to_owned())?;
        if let Err(error) = connection.send(request) {
            return Err(self.failed(error));
        }
        Ok(())
    }

    fn answered(&mut self, answer: &Answer) -> Result<&str, String> {
        self.answer = serde_json::to_string(answer).map_err(|error| error.to_string())?;
        Ok(&self.answer)
    }

    /// The disks connected that may hold backups: `{"disks": [{"uuid", "label", "fsType",
    /// "size"}]}`, or `{"refused": {"reason"}}`.
    pub fn disks(&mut self) -> Result<&str, String> {
        self.send(&Request::Disks {})?;
        match self.receive(DISKS_WAIT)? {
            answer @ (Answer::Disks(_) | Answer::Refused { .. }) => self.answered(&answer),
            _ => Err(self.failed("not an answer to disks")),
        }
    }

    /// The computers `password` opens at `source` (JSON: `{"disk": {"uuid"}}` or `{"share":
    /// {"share": {"host", "share", "user"}}}`): `{"backups": {"computers", "unopened"}}`, or
    /// `{"refused": {"reason"}}`.
    pub fn backups(
        &mut self,
        source: &str,
        password: &Secret,
        share_password: Option<&Secret>,
    ) -> Result<&str, String> {
        let source: Source =
            serde_json::from_str(source).map_err(|error| format!("not a source: {error}"))?;
        self.send(&Request::Backups {
            source: &source,
            password,
            share_password,
        })?;
        match self.receive(BACKUPS_WAIT)? {
            answer @ (Answer::Backups(_) | Answer::Refused { .. }) => self.answered(&answer),
            _ => Err(self.failed("not an answer to backups")),
        }
    }

    /// Asks for `snapshot` of the computer whose repository is `repository` to be migrated: its
    /// progress and answer arrive by [`Migration::follow`].
    pub fn migrate(&mut self, repository: &str, snapshot: &str) -> Result<(), String> {
        self.send(&Request::Migrate {
            repository,
            snapshot,
        })?;
        self.migrating = true;
        Ok(())
    }

    /// The migration's next message, waiting for it: `{"migrating": {"step", "percent"}}` while it
    /// goes on, then `{"migrated": {"computer", "accounts"}}` or `{"refused": {"reason"}}`.
    pub fn follow(&mut self) -> Result<&str, String> {
        if !self.migrating {
            return Err("no migration was asked for".into());
        }
        let answer = match self.receive(MIGRATE_WAIT) {
            Ok(answer) => answer,
            Err(error) => {
                self.migrating = false;
                return Err(error);
            }
        };
        match &answer {
            Answer::Migrating { step, percent }
                if matches!(step.as_str(), "accounts" | "files" | "finishing")
                    && (0.0..=1.0).contains(percent) => {}
            Answer::Migrated { .. } | Answer::Refused { .. } => self.migrating = false,
            _ => {
                self.migrating = false;
                return Err(self.failed("not an answer to a migration"));
            }
        }
        self.answered(&answer)
    }
}

// ---------------------------------------------------------------------------------------------
// The C ABI

/// A conversation with the migration service.
pub struct OxyOSMigration(Migration);

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

/// # Safety
/// `out` points to `capacity` writable bytes, or is null with `capacity` 0.
#[allow(unsafe_code)]
unsafe fn answered(result: Result<&str, String>, out: *mut c_char, capacity: usize) -> i64 {
    match result {
        // SAFETY: the caller's contract.
        Ok(json) => unsafe { write_out(json.as_bytes(), out, capacity) },
        Err(error) => i64::from(fail(error)),
    }
}

/// # Safety
/// `text` is null or a NUL-terminated string valid for the call.
#[allow(unsafe_code)]
unsafe fn text_arg<'a>(text: *const c_char) -> Option<&'a str> {
    if text.is_null() {
        return None;
    }
    // SAFETY: the caller's contract.
    std::str::from_utf8(unsafe { CStr::from_ptr(text) }.to_bytes()).ok()
}

/// # Safety
/// As `text_arg`.
#[allow(unsafe_code)]
unsafe fn secret_arg(text: *const c_char) -> Option<Secret> {
    // SAFETY: the caller's contract.
    unsafe { text_arg(text) }.map(Secret::new)
}

/// Whether this computer migrates from a backup: its migration service's socket is there (systemd
/// makes it with `oxyos-migrate.socket`, before any conversation starts the service). Where it is
/// not, setup offers no Transfer Your Data.
pub fn available_at(socket: &std::path::Path) -> bool {
    use std::os::unix::fs::FileTypeExt;
    std::fs::metadata(socket).is_ok_and(|m| m.file_type().is_socket())
}

/// 1 when this computer migrates from a backup (its migration service's socket is there), else 0.
#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn oxyos_migration_available() -> i32 {
    i32::from(available_at(&socket()))
}

/// Opens a conversation. NULL when the service cannot be reached (`oxyos_last_error`).
#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn oxyos_migration_new() -> *mut OxyOSMigration {
    match Migration::connect(&socket()) {
        Ok(migration) => Box::into_raw(Box::new(OxyOSMigration(migration))),
        Err(error) => {
            fail(error);
            std::ptr::null_mut()
        }
    }
}

/// The disks: their JSON's length, or OXYOS_ERROR (`oxyos_last_error`): the conversation ended.
///
/// # Safety
/// `migration` is one `oxyos_migration_new` returned, not freed; `out` points to `capacity`
/// writable bytes, or is null with `capacity` 0.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_migration_disks(
    migration: *mut OxyOSMigration,
    out: *mut c_char,
    capacity: usize,
) -> i64 {
    if migration.is_null() || (out.is_null() && capacity != 0) {
        return i64::from(fail("null migration or buffer"));
    }
    // SAFETY: the caller's contract.
    let migration = &mut unsafe { &mut *migration }.0;
    let result = migration.disks();
    // SAFETY: the caller's contract.
    unsafe { answered(result, out, capacity) }
}

/// The computers `password` opens at `source` (JSON), `share_password` a share's or null; both
/// secrets are copied into buffers this library overwrites, the caller overwrites its own.
///
/// # Safety
/// As `oxyos_migration_disks`; the strings are NUL-terminated (or `share_password` null) and valid
/// for the call.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_migration_backups(
    migration: *mut OxyOSMigration,
    source: *const c_char,
    password: *const c_char,
    share_password: *const c_char,
    out: *mut c_char,
    capacity: usize,
) -> i64 {
    if migration.is_null() || (out.is_null() && capacity != 0) {
        return i64::from(fail("null migration or buffer"));
    }
    // SAFETY: the caller's contract.
    let (source, password, share_password) = unsafe {
        (
            text_arg(source),
            secret_arg(password),
            secret_arg(share_password),
        )
    };
    let (Some(source), Some(password)) = (source, password) else {
        return i64::from(fail("the backups need a source and a password"));
    };
    // SAFETY: the caller's contract.
    let migration = &mut unsafe { &mut *migration }.0;
    let result = migration.backups(source, &password, share_password.as_ref());
    // SAFETY: the caller's contract.
    unsafe { answered(result, out, capacity) }
}

/// Asks for a backup to be migrated; its messages by `oxyos_migration_next`. 0, or OXYOS_ERROR.
///
/// # Safety
/// `migration` as `oxyos_migration_disks`; the strings are NUL-terminated and valid for the call.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_migration_migrate(
    migration: *mut OxyOSMigration,
    repository: *const c_char,
    snapshot: *const c_char,
) -> c_int {
    if migration.is_null() {
        return fail("null migration");
    }
    // SAFETY: the caller's contract.
    let (Some(repository), Some(snapshot)) = (unsafe { text_arg(repository) }, unsafe {
        text_arg(snapshot)
    }) else {
        return fail("a migration needs a repository and a backup");
    };
    // SAFETY: the caller's contract.
    let migration = &mut unsafe { &mut *migration }.0;
    match migration.migrate(repository, snapshot) {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

/// The migration's next message, waiting for it (never on the interface's thread).
///
/// # Safety
/// As `oxyos_migration_disks`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_migration_next(
    migration: *mut OxyOSMigration,
    out: *mut c_char,
    capacity: usize,
) -> i64 {
    if migration.is_null() || (out.is_null() && capacity != 0) {
        return i64::from(fail("null migration or buffer"));
    }
    // SAFETY: the caller's contract.
    let migration = &mut unsafe { &mut *migration }.0;
    let result = migration.follow();
    // SAFETY: the caller's contract.
    unsafe { answered(result, out, capacity) }
}

/// The last answer again, for a buffer that was too small.
///
/// # Safety
/// As `oxyos_migration_disks`.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_migration_answer(
    migration: *const OxyOSMigration,
    out: *mut c_char,
    capacity: usize,
) -> i64 {
    if migration.is_null() || (out.is_null() && capacity != 0) {
        return i64::from(fail("null migration or buffer"));
    }
    // SAFETY: the caller's contract.
    let answer = &unsafe { &*migration }.0.answer;
    // SAFETY: the caller's contract.
    unsafe { write_out(answer.as_bytes(), out, capacity) }
}

/// Ends the conversation (a disk the service mounted for it is unmounted). A migration under way
/// is finished by the service all the same.
///
/// # Safety
/// `migration` is one `oxyos_migration_new` returned, or null; not used again.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_migration_free(migration: *mut OxyOSMigration) {
    if !migration.is_null() {
        // SAFETY: the caller's contract: it came from Box::into_raw.
        drop(unsafe { Box::from_raw(migration) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyos_install::socket::Listener;
    use serde_json::{json, Value};

    /// A directory of the test's own for a socket (a socket's path holds at most 107 bytes).
    fn socket_path() -> (PathBuf, Removed) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "oxyos-ffi-migrate-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        (dir.join("socket"), Removed(dir))
    }

    struct Removed(PathBuf);

    impl Drop for Removed {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A service answering each message with `answer(message)`'s messages; what it received.
    fn service(
        socket: &Path,
        answer: impl Fn(&Value) -> Vec<Value> + Send + 'static,
    ) -> std::thread::JoinHandle<Vec<Value>> {
        let listener = Listener::bind(socket, 0o600).unwrap();
        std::thread::spawn(move || {
            let connection = listener.accept().unwrap();
            let mut received = Vec::new();
            while let Ok(Some(Some(message))) = connection.receive::<Value>() {
                for answer in answer(&message) {
                    connection.send(&answer).unwrap();
                }
                received.push(message);
            }
            received
        })
    }

    #[test]
    fn it_is_available_only_where_its_socket_is() {
        let dir =
            std::env::temp_dir().join(format!("oxyos-migrate-available-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("socket");
        assert!(!available_at(&path));
        std::fs::write(&path, "").unwrap();
        assert!(!available_at(&path), "a file is no socket");
        std::fs::remove_file(&path).unwrap();
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert!(available_at(&path));
        drop(listener);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_conversation_lists_opens_and_migrates_reading_every_answer_strictly() {
        let (socket, _dir) = socket_path();
        let served = service(&socket, |message| match message {
            m if m.get("disks").is_some() => vec![json!({"disks": [
                {"uuid": "1234-ABCD", "label": "Backups", "fsType": "ext4", "size": 1000}]})],
            m if m.get("backups").is_some() => vec![json!({"backups": {"computers": [
                {"repository": "0123456789abcdef", "name": "ada-laptop",
                 "backups": [{"snapshot": "0123abcd", "time": 1_794_000_000}],
                 "accounts": [{"user": "oxy-a", "name": "Ada", "administrator": true}]}],
                "unopened": 1}})],
            m if m.get("migrate").is_some() => vec![
                json!({"migrating": {"step": "accounts", "percent": 0.0}}),
                json!({"migrating": {"step": "files", "percent": 0.5}}),
                json!({"migrated": {"computer": "ada-laptop",
                                    "accounts": [{"user": "oxy-a", "name": "Ada"}]}}),
            ],
            _ => vec![json!({"refused": {"reason": "invalidRequest"}})],
        });
        let mut migration = Migration::connect(&socket).unwrap();
        let disks: Value = serde_json::from_str(migration.disks().unwrap()).unwrap();
        assert_eq!(disks["disks"][0]["fsType"], "ext4");
        assert!(
            migration.follow().is_err(),
            "nothing to follow before a migration"
        );
        // A source that is not one is never sent.
        assert!(migration
            .backups(r#"{"disk": {"uuid": 1}}"#, &Secret::new("p"), None)
            .is_err());
        let backups: Value = serde_json::from_str(
            migration
                .backups(
                    r#"{"disk": {"uuid": "1234-ABCD"}}"#,
                    &Secret::new("correct horse"),
                    None,
                )
                .unwrap(),
        )
        .unwrap();
        assert_eq!(backups["backups"]["computers"][0]["name"], "ada-laptop");
        assert_eq!(backups["backups"]["unopened"], 1);
        migration.migrate("0123456789abcdef", "0123abcd").unwrap();
        assert!(migration.disks().is_err(), "nothing else while it migrates");
        let mut said = Vec::new();
        loop {
            let message: Value = serde_json::from_str(migration.follow().unwrap()).unwrap();
            let done = message.get("migrating").is_none();
            said.push(message);
            if done {
                break;
            }
        }
        assert_eq!(said.len(), 3);
        assert_eq!(said[2]["migrated"]["accounts"][0]["user"], "oxy-a");
        drop(migration);
        let received = served.join().unwrap();
        assert_eq!(
            received[1],
            json!({"backups": {"source": {"disk": {"uuid": "1234-ABCD"}}, "password": "correct horse"}})
        );
        assert_eq!(
            received[2],
            json!({"migrate": {"repository": "0123456789abcdef", "snapshot": "0123abcd"}})
        );
    }

    #[test]
    fn an_answer_out_of_place_ends_the_conversation() {
        let (socket, _dir) = socket_path();
        let _served = service(&socket, |_| {
            vec![json!({"migrated": {"computer": "x", "accounts": []}})]
        });
        let mut migration = Migration::connect(&socket).unwrap();
        assert!(migration.disks().is_err());
        assert!(migration.disks().is_err(), "the conversation has ended");
    }

    #[test]
    fn the_headers_message_is_the_sockets() {
        let header = include_str!("../include/oxyos_client.h");
        let defined = header
            .lines()
            .find_map(|line| line.strip_prefix("#define OXYOS_MIGRATION_MESSAGE_MAX "))
            .expect("OXYOS_MIGRATION_MESSAGE_MAX is defined");
        assert_eq!(
            defined.trim().parse::<usize>().unwrap(),
            oxyos_install::socket::MAX_MESSAGE
        );
        for declared in [
            "oxyos_migration_new(",
            "oxyos_migration_disks(",
            "oxyos_migration_backups(",
            "oxyos_migration_migrate(",
            "oxyos_migration_next(",
            "oxyos_migration_answer(",
            "oxyos_migration_free(",
        ] {
            assert!(
                header.contains(declared),
                "{declared} is not in oxyos_client.h"
            );
        }
    }

    #[test]
    fn the_c_abi_writes_answers_and_says_why_it_cannot() {
        let (socket, _dir) = socket_path();
        let _served = service(&socket, |_| {
            vec![json!({"refused": {"reason": "alreadySetUp"}})]
        });
        let mut migration = OxyOSMigration(Migration::connect(&socket).unwrap());
        let mut out = vec![0 as c_char; 8];
        // SAFETY: `migration` lives, and `out` has its length.
        #[allow(unsafe_code)]
        let length = unsafe { oxyos_migration_disks(&mut migration, out.as_mut_ptr(), out.len()) };
        let expected = r#"{"refused":{"reason":"alreadySetUp"}}"#;
        assert_eq!(length as usize, expected.len());
        let mut out = vec![0 as c_char; 64];
        // SAFETY: as above.
        #[allow(unsafe_code)]
        let length = unsafe { oxyos_migration_answer(&migration, out.as_mut_ptr(), out.len()) };
        // SAFETY: the library wrote a NUL-terminated string that fits.
        #[allow(unsafe_code)]
        let written = unsafe { CStr::from_ptr(out.as_ptr()) }.to_str().unwrap();
        assert_eq!((length as usize, written), (expected.len(), expected));
    }
}
