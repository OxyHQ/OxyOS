//! restic, the engine every backup goes through ([ADR 0059](../../docs/adr/0059-backups.mdx),
//! decision 1): deduplicated, compressed, encrypted and authenticated snapshots, checked by its own
//! `check`, thinned by its own `forget`. This module runs Debian's `restic` program from an
//! argument vector, never a shell, with a clean environment, and reads what it says in JSON
//! (`--json`). Nothing here reads or writes a repository's files itself.
//!
//! - **The repository's password** is given in `RESTIC_PASSWORD`, restic's documented way. The
//!   environment of a process is readable by its own user and root alone (`/proc/PID/environ`,
//!   mode 0400, behind ptrace's check), and the service is root; nothing is written to a file.
//! - **A share** is reached through rclone's SMB client, which restic starts itself (`rclone:`),
//!   configured by environment variables alone ([`crate::share`]).
//! - **Stopping** is restic's own: `SIGINT`, after which it removes its lock and exits; a backup
//!   stopped keeps nothing half made (restic writes the snapshot last), and the next one reads
//!   only what changed since the last snapshot, deduplicated against everything already stored.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use zeroize::Zeroizing;

/// Debian's restic.
pub const RESTIC: &str = "/usr/bin/restic";
/// Debian's rclone, the SMB client restic starts for a share.
pub const RCLONE: &str = "/usr/bin/rclone";

/// What went wrong, as restic says it (its exit status first, then its words).
#[derive(Debug)]
pub enum Error {
    /// No repository there (exit status 10).
    NoRepository,
    /// The password opens none of the repository's keys (exit status 12).
    WrongPassword,
    /// Another process holds the repository's lock (exit status 11).
    Locked,
    /// The destination is full.
    NoSpace,
    /// The destination refused the credentials it was given (a share's user and password).
    Refused,
    /// The destination cannot be reached (a share's computer is not there).
    Unreachable,
    /// Stopped, as asked.
    Stopped,
    /// Some items could not be read (exit status 3): the snapshot holds the rest.
    Incomplete,
    /// The program could not be started (it is not installed, or cannot be run).
    Program(io::Error),
    /// Anything else, in restic's words.
    Failed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NoRepository => f.write_str("there is no backup there"),
            Error::WrongPassword => f.write_str("the password is not the backup's"),
            Error::Locked => f.write_str("the backup is in use by another program"),
            Error::NoSpace => f.write_str("the destination is full"),
            Error::Refused => f.write_str("the destination refused the credentials"),
            Error::Unreachable => f.write_str("the destination cannot be reached"),
            Error::Stopped => f.write_str("stopped"),
            Error::Incomplete => f.write_str("some items could not be read"),
            Error::Program(error) => write!(f, "restic cannot be run: {error}"),
            Error::Failed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Where a repository is, as restic names it, and what restic's environment needs to reach it.
#[derive(Clone)]
pub struct Location {
    /// `RESTIC_REPOSITORY`: a directory, or `rclone:REMOTE:PATH`.
    pub repository: OsString,
    /// Variables for rclone, which restic passes on (a share's configuration).
    pub environment: Vec<(String, Zeroizing<String>)>,
}

impl Location {
    /// A repository in a directory (a disk's, mounted).
    pub fn directory(path: &Path) -> Self {
        Self {
            repository: path.as_os_str().to_owned(),
            environment: Vec::new(),
        }
    }
}

impl fmt::Debug for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The environment holds a share's password: never shown.
        f.debug_struct("Location")
            .field("repository", &self.repository)
            .finish_non_exhaustive()
    }
}

/// A repository and the password that opens it.
#[derive(Clone)]
pub struct Repository {
    pub location: Location,
    pub password: Zeroizing<String>,
}

impl fmt::Debug for Repository {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Repository")
            .field("location", &self.location)
            .finish_non_exhaustive()
    }
}

/// How a running restic is stopped from another thread.
#[derive(Clone, Default)]
pub struct Stop {
    inner: Arc<StopInner>,
}

#[derive(Default)]
struct StopInner {
    asked: AtomicBool,
    child: Mutex<Option<u32>>,
}

impl Stop {
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks the running program, if any, to stop (`SIGINT`); one started later stops at once.
    pub fn stop(&self) {
        self.inner.asked.store(true, Ordering::SeqCst);
        let child = self.inner.child.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pid) = *child {
            interrupt(pid);
        }
    }

    pub fn asked(&self) -> bool {
        self.inner.asked.load(Ordering::SeqCst)
    }

    fn started(&self, pid: u32) {
        let mut child = self.inner.child.lock().unwrap_or_else(|e| e.into_inner());
        *child = Some(pid);
        if self.asked() {
            interrupt(pid);
        }
    }

    fn ended(&self) {
        *self.inner.child.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

fn interrupt(pid: u32) {
    if let Some(pid) = i32::try_from(pid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    {
        let _ = rustix::process::kill_process(pid, rustix::process::Signal::INT);
    }
}

/// How far a backup or a restore is, as restic says it once a second (a restore's counts are
/// `files_restored` and `bytes_restored`).
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Progress {
    #[serde(default)]
    pub percent_done: f64,
    #[serde(default)]
    pub total_files: u64,
    #[serde(default, alias = "files_restored")]
    pub files_done: u64,
    #[serde(default)]
    pub total_bytes: u64,
    #[serde(default, alias = "bytes_restored")]
    pub bytes_done: u64,
    #[serde(default)]
    pub seconds_remaining: Option<u64>,
}

/// What a finished backup made.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Summary {
    pub snapshot_id: String,
    #[serde(default)]
    pub files_new: u64,
    #[serde(default)]
    pub files_changed: u64,
    #[serde(default)]
    pub data_added_packed: u64,
    #[serde(default)]
    pub total_files_processed: u64,
    #[serde(default)]
    pub total_bytes_processed: u64,
    /// Items that could not be read (restic's exit status 3): the snapshot holds the rest.
    #[serde(skip)]
    pub unreadable: Vec<String>,
}

/// A snapshot, as `restic snapshots` lists it.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub time: String,
    #[serde(default)]
    pub hostname: String,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Snapshot {
    /// Its time in seconds since the epoch.
    pub fn seconds(&self) -> Option<i64> {
        seconds(&self.time)
    }
}

/// RFC 3339, as restic writes times, in seconds since the epoch.
pub fn seconds(time: &str) -> Option<i64> {
    time.parse::<jiff::Timestamp>().ok().map(|t| t.as_second())
}

/// An item of a snapshot, as `restic ls` and `restic find` list it.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Node {
    pub path: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub mtime: Option<String>,
    #[serde(default)]
    pub uid: Option<u32>,
}

/// The versions of an item: the snapshots that hold it, newest first.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub snapshot: String,
    pub node: Node,
}

/// What a repository takes on its destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct Stats {
    /// Bytes stored, compressed and encrypted.
    pub total_size: u64,
    #[serde(default)]
    pub snapshots_count: u64,
}

/// What a backup reads and leaves out.
#[derive(Clone, Debug, Default)]
pub struct Backup {
    /// What is read: absolute paths.
    pub paths: Vec<PathBuf>,
    /// restic's exclude patterns.
    pub excludes: Vec<String>,
    /// The snapshot's host: the computer's backup name.
    pub host: String,
    pub tags: Vec<String>,
    /// The snapshot's time (`YYYY-MM-DD HH:MM:SS`, local), when it is not now: a restore's
    /// tests, and backups of another time.
    pub time: Option<String>,
}

/// The policy restic's `forget` thins with (`--keep-*`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keep {
    pub arguments: Vec<String>,
}

/// restic, and where it keeps its cache.
#[derive(Clone, Debug)]
pub struct Engine {
    restic: PathBuf,
    rclone: PathBuf,
    cache: PathBuf,
}

impl Engine {
    /// Debian's programs, with `cache` for restic's cache of repositories' indexes and trees.
    pub fn new(cache: &Path) -> Self {
        Self {
            restic: RESTIC.into(),
            rclone: RCLONE.into(),
            cache: cache.to_owned(),
        }
    }

    /// Other programs (the tests' and a developer's, found on `PATH`).
    pub fn with_programs(mut self, restic: &Path, rclone: &Path) -> Self {
        self.restic = restic.to_owned();
        self.rclone = rclone.to_owned();
        self
    }

    /// The programs found on `PATH`, if restic is there.
    pub fn from_path(cache: &Path) -> Option<Self> {
        let find = |name: &str| {
            std::env::var_os("PATH").and_then(|path| {
                std::env::split_paths(&path)
                    .map(|d| d.join(name))
                    .find(|p| p.is_file())
            })
        };
        let restic = find("restic")?;
        let rclone = find("rclone").unwrap_or_else(|| RCLONE.into());
        Some(Self::new(cache).with_programs(&restic, &rclone))
    }

    pub fn restic(&self) -> &Path {
        &self.restic
    }

    pub fn rclone(&self) -> &Path {
        &self.rclone
    }

    fn command(&self, repository: &Repository, arguments: &[&str]) -> Command {
        let mut command = Command::new(&self.restic);
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("LANG", "C.UTF-8")
            .env("HOME", &self.cache)
            .env("RESTIC_CACHE_DIR", &self.cache)
            .env("RESTIC_REPOSITORY", &repository.location.repository)
            .env("RESTIC_PASSWORD", repository.password.as_str())
            // A status line a second while a backup runs, never more.
            .env("RESTIC_PROGRESS_FPS", "1");
        for (name, value) in &repository.location.environment {
            command.env(name, value.as_str());
        }
        if is_rclone(&repository.location) {
            command
                .arg("--option")
                .arg(format!("rclone.program={}", self.rclone.display()));
        }
        command.arg("--json").args(arguments);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    fn spawn(&self, mut command: Command, stop: Option<&Stop>) -> Result<Running> {
        if stop.is_some_and(Stop::asked) {
            return Err(Error::Stopped);
        }
        let mut child = command.spawn().map_err(Error::Program)?;
        if let Some(stop) = stop {
            stop.started(child.id());
        }
        let stderr = child.stderr.take().map(|stderr| {
            std::thread::Builder::new()
                .name("oxyos-backup-restic-stderr".into())
                .spawn(move || {
                    let mut text = String::new();
                    let _ = BufReader::new(stderr)
                        .take(1 << 20)
                        .read_to_string(&mut text);
                    text
                })
        });
        Ok(Running {
            child,
            stderr: stderr.and_then(|s| s.ok()),
            stop: stop.cloned(),
        })
    }

    /// Runs a command to its end and gives what it printed on its standard output.
    fn run(
        &self,
        repository: &Repository,
        arguments: &[&str],
        stop: Option<&Stop>,
    ) -> Result<String> {
        let command = self.command(repository, arguments);
        let mut running = self.spawn(command, stop)?;
        let mut output = String::new();
        if let Some(stdout) = running.child.stdout.as_mut() {
            stdout
                .read_to_string(&mut output)
                .map_err(|e| Error::Failed(e.to_string()))?;
        }
        running.finish().map(|_| output)
    }

    /// Makes a repository at `repository`'s location, opened by its password, and gives it a
    /// second key: `second`, the person's backup password, which opens it anywhere else.
    pub fn init(&self, repository: &Repository, second: Option<&str>) -> Result<()> {
        self.run(repository, &["init"], None)?;
        if let Some(second) = second {
            self.add_key(repository, second)?;
        }
        Ok(())
    }

    /// Adds a key to the repository: `password` opens it from then on, as its own does.
    pub fn add_key(&self, repository: &Repository, password: &str) -> Result<()> {
        let mut command = self.command(
            repository,
            &["key", "add", "--new-password-file", "/dev/stdin"],
        );
        command.stdin(Stdio::piped());
        let mut running = self.spawn(command, None)?;
        if let Some(mut stdin) = running.child.stdin.take() {
            stdin
                .write_all(password.as_bytes())
                .map_err(|e| Error::Failed(e.to_string()))?;
        }
        let mut sink = String::new();
        if let Some(stdout) = running.child.stdout.as_mut() {
            let _ = stdout.read_to_string(&mut sink);
        }
        running.finish().map(|_| ())
    }

    /// Whether the repository is there and its password opens it.
    pub fn open(&self, repository: &Repository) -> Result<()> {
        self.run(repository, &["cat", "config", "--no-lock"], None)
            .map(|_| ())
    }

    /// Backs up, telling `progress` as restic says it, until it ends or `stop` stops it.
    pub fn backup(
        &self,
        repository: &Repository,
        backup: &Backup,
        stop: &Stop,
        mut progress: impl FnMut(&Progress),
    ) -> Result<Summary> {
        let mut arguments: Vec<String> = vec![
            "backup".into(),
            // Never into another file system mounted below what is read (a disk, a share, the
            // backup disk itself).
            "--one-file-system".into(),
            // Folders marked as caches (CACHEDIR.TAG), which their programs make again.
            "--exclude-caches".into(),
            "--host".into(),
            backup.host.clone(),
        ];
        if let Some(time) = &backup.time {
            arguments.push("--time".into());
            arguments.push(time.clone());
        }
        for tag in &backup.tags {
            arguments.push("--tag".into());
            arguments.push(tag.clone());
        }
        for exclude in &backup.excludes {
            arguments.push("--exclude".into());
            arguments.push(exclude.clone());
        }
        arguments.push("--".into());
        arguments.extend(backup.paths.iter().map(|p| p.display().to_string()));
        let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
        let command = self.command(repository, &arguments);
        let mut running = self.spawn(command, Some(stop))?;
        let mut summary = None;
        let mut unreadable = Vec::new();
        if let Some(stdout) = running.child.stdout.take() {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(message) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };
                match message.get("message_type").and_then(|t| t.as_str()) {
                    Some("status") => {
                        if let Ok(status) = serde_json::from_value::<Progress>(message) {
                            progress(&status);
                        }
                    }
                    Some("summary") => {
                        summary = serde_json::from_value::<Summary>(message).ok();
                    }
                    Some("error") => {
                        if let Some(item) = message.get("item").and_then(|i| i.as_str()) {
                            if unreadable.len() < 100 {
                                unreadable.push(item.to_owned());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        match (running.finish(), summary) {
            (Ok(()) | Err(Error::Incomplete), Some(mut summary)) => {
                summary.unreadable = unreadable;
                Ok(summary)
            }
            (Ok(()) | Err(Error::Incomplete), None) => {
                Err(Error::Failed("restic said nothing of the snapshot".into()))
            }
            (Err(error), _) => Err(error),
        }
    }

    /// The repository's snapshots of `host`, oldest first.
    pub fn snapshots(&self, repository: &Repository, host: Option<&str>) -> Result<Vec<Snapshot>> {
        let mut arguments = vec!["snapshots", "--no-lock"];
        if let Some(host) = host {
            arguments.extend(["--host", host]);
        }
        let output = self.run(repository, &arguments, None)?;
        let mut snapshots: Vec<Snapshot> =
            serde_json::from_str(output.trim()).map_err(|e| Error::Failed(e.to_string()))?;
        snapshots.sort_by_key(|s| s.seconds().unwrap_or(0));
        Ok(snapshots)
    }

    /// Thins `host`'s snapshots by `keep` (restic's own policy): the ids it removed.
    pub fn forget(&self, repository: &Repository, host: &str, keep: &Keep) -> Result<Vec<String>> {
        let mut arguments = vec!["forget", "--host", host, "--group-by", "host"];
        arguments.extend(keep.arguments.iter().map(String::as_str));
        let output = self.run(repository, &arguments, None)?;
        #[derive(Deserialize)]
        struct Group {
            #[serde(default)]
            remove: Option<Vec<Snapshot>>,
        }
        let groups: Vec<Group> =
            serde_json::from_str(output.trim()).map_err(|e| Error::Failed(e.to_string()))?;
        Ok(groups
            .into_iter()
            .flat_map(|g| g.remove.unwrap_or_default())
            .map(|s| s.id)
            .collect())
    }

    /// Removes these snapshots (the oldest, when the destination is full).
    pub fn forget_ids(&self, repository: &Repository, ids: &[String]) -> Result<()> {
        let mut arguments = vec!["forget"];
        arguments.extend(ids.iter().map(String::as_str));
        self.run(repository, &arguments, None).map(|_| ())
    }

    /// Removes the data no snapshot needs any more.
    pub fn prune(&self, repository: &Repository, stop: &Stop) -> Result<()> {
        self.run(repository, &["prune"], Some(stop)).map(|_| ())
    }

    /// Checks the repository's structure and reads back `subset` of its data (restic's
    /// `--read-data-subset`, e.g. `5%`).
    pub fn check(&self, repository: &Repository, subset: Option<&str>, stop: &Stop) -> Result<()> {
        let mut arguments = vec!["check"];
        let subset_argument;
        if let Some(subset) = subset {
            subset_argument = format!("--read-data-subset={subset}");
            arguments.push(&subset_argument);
        }
        self.run(repository, &arguments, Some(stop)).map(|_| ())
    }

    /// Removes locks left by restic processes that no longer run.
    pub fn unlock(&self, repository: &Repository) -> Result<()> {
        self.run(repository, &["unlock"], None).map(|_| ())
    }

    /// What the repository takes on its destination.
    pub fn stats(&self, repository: &Repository) -> Result<Stats> {
        let output = self.run(
            repository,
            &["stats", "--no-lock", "--mode", "raw-data"],
            None,
        )?;
        serde_json::from_str(output.trim()).map_err(|e| Error::Failed(e.to_string()))
    }

    /// Every version of the item at `path` (absolute, as it was backed up) in `host`'s
    /// snapshots, newest first.
    pub fn find(&self, repository: &Repository, host: &str, path: &str) -> Result<Vec<Found>> {
        let output = self.run(
            repository,
            &["find", "--no-lock", "--host", host, "--", path],
            None,
        )?;
        #[derive(Deserialize)]
        struct Matches {
            snapshot: String,
            #[serde(default)]
            matches: Vec<Node>,
        }
        let found: Vec<Matches> = if output.trim().is_empty() {
            Vec::new()
        } else {
            serde_json::from_str(output.trim()).map_err(|e| Error::Failed(e.to_string()))?
        };
        Ok(found
            .into_iter()
            .flat_map(|m| {
                let snapshot = m.snapshot;
                m.matches
                    .into_iter()
                    .filter(|node| node.path == path)
                    .map(move |node| Found {
                        snapshot: snapshot.clone(),
                        node,
                    })
            })
            .collect())
    }

    /// The item at `path` in `snapshot` and, if it is a folder, what it held (not what its
    /// folders hold).
    pub fn ls(&self, repository: &Repository, snapshot: &str, path: &str) -> Result<Vec<Node>> {
        let output = self.run(repository, &["ls", "--no-lock", snapshot, "--", path], None)?;
        let mut nodes = Vec::new();
        for line in output.lines() {
            let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if message.get("struct_type").and_then(|t| t.as_str()) != Some("node") {
                continue;
            }
            if let Ok(node) = serde_json::from_value::<Node>(message) {
                nodes.push(node);
            }
        }
        Ok(nodes)
    }

    /// The item at `path` in `snapshot`, as it is read: a file's bytes, or a folder as a tar
    /// archive. The caller reads it to its end, then [`Dump::finish`]es it.
    pub fn dump(
        &self,
        repository: &Repository,
        snapshot: &str,
        path: &str,
        folder: bool,
    ) -> Result<Dump> {
        let mut arguments = vec!["dump", "--no-lock"];
        if folder {
            arguments.extend(["--archive", "tar"]);
        }
        arguments.extend([snapshot, "--", path]);
        let command = self.command(repository, &arguments);
        let mut running = self.spawn(command, None)?;
        let stdout = running
            .child
            .stdout
            .take()
            .ok_or_else(|| Error::Failed("restic's output".into()))?;
        Ok(Dump { stdout, running })
    }

    /// Restores `snapshot`'s `includes` (absolute paths, as they were backed up) into `target`,
    /// owners and modes as they were: the recovery's and the migration's, as root, into a
    /// system's root. `snapshot` may name a folder of it (`ID:/folder`, restic's own), whose
    /// contents are then restored into `target` and the includes are read from it.
    pub fn restore(
        &self,
        repository: &Repository,
        snapshot: &str,
        includes: &[String],
        target: &Path,
        stop: &Stop,
        mut progress: impl FnMut(&Progress),
    ) -> Result<()> {
        let target = target.display().to_string();
        let mut arguments = vec!["restore", snapshot, "--target", target.as_str()];
        for include in includes {
            arguments.push("--include");
            arguments.push(include);
        }
        let command = self.command(repository, &arguments);
        let mut running = self.spawn(command, Some(stop))?;
        if let Some(stdout) = running.child.stdout.take() {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(message) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };
                if message.get("message_type").and_then(|t| t.as_str()) == Some("status") {
                    if let Ok(status) = serde_json::from_value::<Progress>(message) {
                        progress(&status);
                    }
                }
            }
        }
        running.finish().map(|_| ())
    }
}

fn is_rclone(location: &Location) -> bool {
    location
        .repository
        .to_str()
        .is_some_and(|r| r.starts_with("rclone:"))
}

/// A running restic.
struct Running {
    child: Child,
    stderr: Option<std::thread::JoinHandle<String>>,
    stop: Option<Stop>,
}

impl Running {
    /// Waits for it, and says how it ended.
    fn finish(mut self) -> Result<()> {
        let status = self.child.wait();
        if let Some(stop) = &self.stop {
            stop.ended();
        }
        let stderr = self
            .stderr
            .take()
            .and_then(|s| s.join().ok())
            .unwrap_or_default();
        let status = status.map_err(Error::Program)?;
        if status.success() {
            return Ok(());
        }
        if self.stop.as_ref().is_some_and(Stop::asked) {
            return Err(Error::Stopped);
        }
        Err(classify(status.code(), &stderr))
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        // A conversation that went away mid-way: restic stops and removes its lock.
        if let Ok(None) = self.child.try_wait() {
            interrupt(self.child.id());
            let _ = self.child.wait();
        }
    }
}

/// How restic ended, from its exit status (restic's documented ones) and what it said; rclone's
/// own words for a share as well.
pub(crate) fn classify(code: Option<i32>, stderr: &str) -> Error {
    let said = said(stderr);
    let lowered = said.to_lowercase();
    // What rclone said of a share comes before restic's last words.
    let everything = stderr.to_lowercase();
    if everything.contains("no space left on device") || everything.contains("disk quota exceeded")
    {
        return Error::NoSpace;
    }
    if [
        "attempted logon is invalid",
        "logon failure",
        "logon_failure",
        "status_access_denied",
        "access denied",
        "authentication failed",
    ]
    .iter()
    .any(|w| everything.contains(w))
    {
        return Error::Refused;
    }
    if [
        "connection refused",
        "no such host",
        "i/o timeout",
        "network is unreachable",
        "no route to host",
        "connection reset",
    ]
    .iter()
    .any(|w| everything.contains(w))
    {
        return Error::Unreachable;
    }
    match code {
        Some(3) => Error::Incomplete,
        Some(10) => Error::NoRepository,
        Some(11) => Error::Locked,
        Some(12) => Error::WrongPassword,
        Some(130) => Error::Stopped,
        _ if lowered.contains("wrong password") => Error::WrongPassword,
        _ if lowered.contains("repository does not exist")
            || lowered.contains("unable to open config file") =>
        {
            Error::NoRepository
        }
        _ => Error::Failed(if said.is_empty() {
            format!("restic ended with status {code:?}")
        } else {
            said
        }),
    }
}

/// restic's last words: its JSON `exit_error`'s message, else its last line.
fn said(stderr: &str) -> String {
    for line in stderr.lines().rev() {
        if let Ok(message) = serde_json::from_str::<BTreeMap<String, serde_json::Value>>(line) {
            if let Some(text) = message.get("message").and_then(|m| m.as_str()) {
                return text.trim().to_owned();
            }
        }
    }
    stderr
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .to_owned()
}

/// An item being read out of a snapshot.
pub struct Dump {
    stdout: ChildStdout,
    running: Running,
}

impl Dump {
    /// Its bytes, as restic writes them.
    pub fn output(&mut self) -> &mut ChildStdout {
        &mut self.stdout
    }

    /// Hands its output over (to be sent to another process), keeping the program to finish.
    pub fn into_parts(self) -> (ChildStdout, DumpEnd) {
        (self.stdout, DumpEnd(self.running))
    }

    /// Waits for restic, once its output was read to its end.
    pub fn finish(self) -> Result<()> {
        drop(self.stdout);
        self.running.finish()
    }
}

/// The program of a [`Dump`] whose output was handed over.
pub struct DumpEnd(Running);

impl DumpEnd {
    pub fn finish(self) -> Result<()> {
        self.0.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restics_exit_statuses_say_what_went_wrong() {
        assert!(matches!(classify(Some(10), ""), Error::NoRepository));
        assert!(matches!(classify(Some(11), ""), Error::Locked));
        assert!(matches!(classify(Some(12), ""), Error::WrongPassword));
        assert!(matches!(classify(Some(3), ""), Error::Incomplete));
        assert!(matches!(
            classify(
                Some(1),
                "{\"message_type\":\"exit_error\",\"code\":1,\"message\":\"Fatal: unable to save snapshot: write /x: no space left on device\"}"
            ),
            Error::NoSpace
        ));
        match classify(Some(1), "something\nFatal: the share went away\n") {
            Error::Failed(message) => assert_eq!(message, "Fatal: the share went away"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn times_are_read_as_restic_writes_them() {
        assert_eq!(
            seconds("2026-11-08T02:47:00.519713011+02:00"),
            Some(1_794_098_820)
        );
        assert_eq!(seconds("yesterday"), None);
    }
}
