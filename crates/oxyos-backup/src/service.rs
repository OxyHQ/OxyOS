//! The backup service ([ADR 0059](../../docs/adr/0059-backups.mdx), decisions 2 to 8): the
//! schedule, the backups, the destinations, and the conversations of its socket ([`crate::ipc`]).
//!
//! - **One thread schedules.** It waits on three descriptors, never on a clock that ticks: a
//!   wake-up pipe (a request, a hold, a change of the configuration), udev's monitor (a disk
//!   connected or taken away) and a `timerfd` on the wall clock set to the next backup's time
//!   (`CLOCK_REALTIME`, absolute, cancelled when the clock is set): a backup due while the
//!   computer slept is due as it wakes, and an idle machine wakes the service once an hour at
//!   most, for the backup itself. Backups run on that thread, one at a time.
//! - **Each conversation has a thread**, as the install service's; a watching one also has a
//!   thread that sends the state as it changes (told by a channel, never polled).
//! - **A destination is used by one maintenance at a time**: backups and reads (browsing,
//!   retrieving: `--no-lock`) share it; thinning, pruning and checking have it alone.
//! - **A disk is mounted while something uses it**, and unmounted when nothing does.

use crate::disks::{self, Disk, Disks};
use crate::engine::{self, Backup, Engine, Location, Repository, Stop};
use crate::ipc::{
    Answer, DestinationStatus, DiskChoice, HoldReason, Item, ItemKind, Request, Running, Status,
    Version,
};
use crate::machine::Manifest;
use crate::policy::{self, Sources};
use crate::share;
use crate::store::{self, Config, Destination, Failure, Keys, Kind, Record, Store};
use oxyos_install::socket::{Connection, Peer};
use rustix::fd::{AsFd, OwnedFd};
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard, RwLock};
use zeroize::Zeroizing;

/// The shortest backup password the service takes.
pub const MIN_PASSWORD: usize = 8;

/// What the service is run with.
pub struct Options {
    /// Its directory ([`store::STATE_DIRECTORY`]).
    pub state: PathBuf,
    pub engine: Engine,
    pub disks: Arc<dyn Disks>,
    pub sources: Sources,
    /// The user database (an administrator, a home).
    pub passwd: PathBuf,
    pub group: PathBuf,
    /// The account service's directory, for the manifest; none in development.
    pub accounts: Option<PathBuf>,
    /// Where the manifest is written before each backup; none in development.
    pub manifest: Option<PathBuf>,
    /// A developer's service: unprivileged, serving its own user alone, who is its administrator
    /// and may browse every source.
    pub development: bool,
    /// The wall clock, in seconds.
    pub clock: Arc<dyn Fn() -> i64 + Send + Sync>,
}

/// The wall clock.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Why a backup ended without one.
enum Ended {
    Failed(&'static str),
    /// Skipped by the person: tried again in an hour.
    Skipped,
    /// Held back (Low Power Mode): tried again once released.
    Held,
    /// The disk was taken away: tried again when it is connected.
    Disconnected,
}

#[derive(Default)]
struct State {
    config: Config,
    records: BTreeMap<String, Record>,
    running: Option<Running>,
    stop: Option<Stop>,
    /// Stopped because a hold arrived, not by the person.
    stopped_for_hold: bool,
    asked: bool,
    holds: usize,
    /// Connected disks, by UUID.
    connected: BTreeMap<String, Disk>,
}

struct Shared {
    options: Options,
    store: Store,
    state: Mutex<State>,
    /// Each watching conversation's channel, by a number of its own.
    watchers: Mutex<Vec<(u64, mpsc::Sender<()>)>>,
    next_watcher: std::sync::atomic::AtomicU64,
    wake_read: OwnedFd,
    wake_write: OwnedFd,
    shutdown: AtomicBool,
    mounts: Mutex<BTreeMap<String, usize>>,
    locks: Mutex<BTreeMap<String, Arc<RwLock<()>>>>,
}

/// The service.
#[derive(Clone)]
pub struct Service {
    shared: Arc<Shared>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl Service {
    /// Opens the service's directory, choosing the computer's name in its snapshots the first
    /// time (`computer`: the machine's host name).
    pub fn open(options: Options, computer: &str) -> io::Result<Self> {
        let store = Store::open(&options.state)?;
        let mut config = store.config()?;
        if config.computer.is_empty() {
            config.computer = if computer.trim().is_empty() {
                "oxyos".into()
            } else {
                computer.trim().to_owned()
            };
            store.put_config(&config)?;
        }
        let records = store.records()?;
        let (wake_read, wake_write) = rustix::pipe::pipe_with(
            rustix::pipe::PipeFlags::CLOEXEC | rustix::pipe::PipeFlags::NONBLOCK,
        )?;
        let connected = options
            .disks
            .list()
            .into_iter()
            .map(|d| (d.uuid.clone(), d))
            .collect();
        Ok(Self {
            shared: Arc::new(Shared {
                options,
                store,
                state: Mutex::new(State {
                    config,
                    records,
                    connected,
                    ..State::default()
                }),
                watchers: Mutex::new(Vec::new()),
                next_watcher: std::sync::atomic::AtomicU64::new(0),
                wake_read,
                wake_write,
                shutdown: AtomicBool::new(false),
                mounts: Mutex::new(BTreeMap::new()),
                locks: Mutex::new(BTreeMap::new()),
            }),
        })
    }

    fn now(&self) -> i64 {
        (self.shared.options.clock)()
    }

    pub(crate) fn wake(&self) {
        let _ = rustix::io::write(&self.shared.wake_write, &[1]);
    }

    /// Ends [`Service::schedule`] (the tests').
    pub fn shutdown(&self) {
        self.shared.shutdown.store(true, Ordering::SeqCst);
        if let Some(stop) = &lock(&self.shared.state).stop {
            stop.stop();
        }
        self.wake();
    }

    fn notify(&self) {
        lock(&self.shared.watchers).retain(|(_, watcher)| watcher.send(()).is_ok());
    }

    // The schedule.

    /// Schedules and runs backups, until [`Service::shutdown`].
    pub fn schedule(&self) {
        let watch = match self.shared.options.disks.watch() {
            Ok(watch) => Some(watch),
            Err(e) => {
                eprintln!("oxyos-backupd: disks cannot be followed: {e}");
                None
            }
        };
        let timer = rustix::time::timerfd_create(
            rustix::time::TimerfdClockId::Realtime,
            rustix::time::TimerfdFlags::CLOEXEC | rustix::time::TimerfdFlags::NONBLOCK,
        )
        .ok();
        loop {
            if self.shared.shutdown.load(Ordering::SeqCst) {
                return;
            }
            self.refresh_disks();
            let now = self.now();
            if let Some((id, asked)) = self.pick(now) {
                self.back_up(&id, asked);
                continue;
            }
            let deadline = self.deadline(now);
            if let Some(timer) = &timer {
                arm(timer, deadline);
            }
            let mut fds = vec![rustix::event::PollFd::new(
                &self.shared.wake_read,
                rustix::event::PollFlags::IN,
            )];
            if let Some(watch) = &watch {
                fds.push(rustix::event::PollFd::from_borrowed_fd(
                    watch.descriptor(),
                    rustix::event::PollFlags::IN,
                ));
            }
            if let Some(timer) = &timer {
                fds.push(rustix::event::PollFd::new(
                    timer,
                    rustix::event::PollFlags::IN,
                ));
            }
            if let Err(e) = rustix::event::poll(&mut fds, None) {
                if e != rustix::io::Errno::INTR {
                    eprintln!("oxyos-backupd: waiting: {e}");
                    return;
                }
            }
            drop(fds);
            let mut buffer = [0u8; 64];
            while rustix::io::read(&self.shared.wake_read, &mut buffer).is_ok_and(|n| n > 0) {}
            if let Some(watch) = &watch {
                watch.changed();
            }
            if let Some(timer) = &timer {
                let mut expirations = [0u8; 8];
                let _ = rustix::io::read(timer, &mut expirations);
            }
        }
    }

    /// Reads which disks are connected; a destination's disk connected again is tried at once.
    fn refresh_disks(&self) {
        let disks: BTreeMap<String, Disk> = self
            .shared
            .options
            .disks
            .list()
            .into_iter()
            .map(|d| (d.uuid.clone(), d))
            .collect();
        let mut state = lock(&self.shared.state);
        let arrived: BTreeSet<String> = disks
            .keys()
            .filter(|uuid| !state.connected.contains_key(*uuid))
            .cloned()
            .collect();
        let changed = disks.keys().ne(state.connected.keys());
        state.connected = disks;
        if !arrived.is_empty() {
            let ids: Vec<String> = state
                .config
                .destinations
                .iter()
                .filter(|d| matches!(&d.kind, Kind::Disk { uuid, .. } if arrived.contains(uuid)))
                .map(|d| d.id.clone())
                .collect();
            for id in ids {
                if let Some(record) = state.records.get_mut(&id) {
                    record.retry_at = None;
                }
            }
        }
        drop(state);
        if changed {
            self.notify();
        }
    }

    fn available(state: &State, destination: &Destination) -> bool {
        match &destination.kind {
            Kind::Disk { uuid, .. } => state.connected.contains_key(uuid),
            Kind::Share(_) => true,
        }
    }

    /// The backup to run now, and whether it was asked for.
    fn pick(&self, now: i64) -> Option<(String, bool)> {
        let mut state = lock(&self.shared.state);
        let available: Vec<&str> = state
            .config
            .destinations
            .iter()
            .filter(|d| Self::available(&state, d))
            .map(|d| d.id.as_str())
            .collect();
        if state.asked {
            let chosen = policy::choose(available, |id| state.records.get(id)).map(str::to_owned);
            state.asked = false;
            return chosen.map(|id| (id, true));
        }
        if !state.config.automatic || state.holds > 0 {
            return None;
        }
        let due = available
            .into_iter()
            .filter(|id| policy::due(state.records.get(*id), now));
        policy::choose(due, |id| state.records.get(id)).map(|id| (id.to_owned(), false))
    }

    /// When the next scheduled backup is due, if any is.
    fn deadline(&self, now: i64) -> Option<i64> {
        let state = lock(&self.shared.state);
        if !state.config.automatic || state.holds > 0 {
            return None;
        }
        state
            .config
            .destinations
            .iter()
            .filter(|d| Self::available(&state, d))
            .map(|d| policy::next(state.records.get(&d.id), now))
            .min()
    }

    fn back_up(&self, id: &str, asked: bool) {
        let stop = Stop::new();
        {
            let mut state = lock(&self.shared.state);
            state.running = Some(Running {
                destination: id.to_owned(),
                phase: "preparing",
                asked,
                percent: 0.0,
                bytes_done: 0,
                bytes_total: 0,
                files_done: 0,
                files_total: 0,
                seconds_remaining: None,
            });
            state.stop = Some(stop.clone());
            state.stopped_for_hold = false;
        }
        self.notify();
        let ended = self.attempt(id, &stop);
        let now = self.now();
        {
            let mut state = lock(&self.shared.state);
            let held = state.stopped_for_hold;
            state.running = None;
            state.stop = None;
            state.stopped_for_hold = false;
            let disconnected = state
                .config
                .destinations
                .iter()
                .find(|d| d.id == id)
                .is_some_and(|d| !Self::available(&state, d));
            let record = state.records.entry(id.to_owned()).or_default();
            match ended {
                Ok(()) => {}
                Err(Ended::Skipped) if held => {}
                Err(Ended::Held) => {}
                Err(Ended::Skipped) => record.retry_at = Some(now + policy::INTERVAL),
                Err(Ended::Disconnected) => record.retry_at = None,
                Err(Ended::Failed(_)) if disconnected => record.retry_at = None,
                Err(Ended::Failed(reason)) => {
                    record.last_failure = Some(Failure {
                        reason: reason.to_owned(),
                        at: now,
                    });
                    record.retry_at = Some(now + policy::RETRY);
                }
            }
            if let Err(e) = self.shared.store.put_records(&state.records) {
                eprintln!("oxyos-backupd: the destinations' records cannot be written: {e}");
            }
        }
        self.notify();
    }

    fn set_phase(&self, phase: &'static str) {
        if let Some(running) = &mut lock(&self.shared.state).running {
            running.phase = phase;
        }
        self.notify();
    }

    fn update(&self, id: &str, change: impl FnOnce(&mut Record)) {
        let mut state = lock(&self.shared.state);
        change(state.records.entry(id.to_owned()).or_default());
        if let Err(e) = self.shared.store.put_records(&state.records) {
            eprintln!("oxyos-backupd: the destinations' records cannot be written: {e}");
        }
    }

    fn attempt(&self, id: &str, stop: &Stop) -> Result<(), Ended> {
        let (config, destination) = {
            let state = lock(&self.shared.state);
            let destination = state
                .config
                .destinations
                .iter()
                .find(|d| d.id == id)
                .cloned()
                .ok_or(Ended::Failed("noDestination"))?;
            (state.config.clone(), destination)
        };
        let ended = |error: engine::Error| -> Ended {
            match error {
                engine::Error::Stopped => {
                    if lock(&self.shared.state).stopped_for_hold {
                        Ended::Held
                    } else {
                        Ended::Skipped
                    }
                }
                error => Ended::Failed(reason(&error, &destination)),
            }
        };
        let destination_lock = self.destination_lock(id);
        let summary = {
            let _shared = destination_lock.read().unwrap_or_else(|e| e.into_inner());
            let opened = self.repository(&destination).map_err(|r| {
                if r == "notConnected" {
                    Ended::Disconnected
                } else {
                    Ended::Failed(r)
                }
            })?;
            if let (Some(directory), false) = (
                &self.shared.options.manifest,
                self.shared.options.development,
            ) {
                let accounts = self.shared.options.accounts.clone().unwrap_or_default();
                match Manifest::of_machine(
                    &config.computer,
                    &accounts,
                    &self.shared.options.passwd,
                    &self.shared.options.group,
                    self.now(),
                ) {
                    Ok(manifest) => {
                        if let Err(e) = manifest.write(directory) {
                            eprintln!("oxyos-backupd: the manifest cannot be written: {e}");
                        }
                    }
                    Err(e) => eprintln!("oxyos-backupd: the manifest cannot be made: {e}"),
                }
            }
            let (paths, excludes) = self.shared.options.sources.with(&config);
            if paths.is_empty() {
                return Err(Ended::Failed("nothingToBackUp"));
            }
            let backup = Backup {
                paths,
                excludes,
                host: config.computer.clone(),
                tags: vec!["oxyos".into()],
                time: None,
            };
            self.set_phase("backingUp");
            let mut tries = 0;
            loop {
                tries += 1;
                let result = self.shared.options.engine.backup(
                    &opened.repository,
                    &backup,
                    stop,
                    |progress| {
                        if let Some(running) = &mut lock(&self.shared.state).running {
                            running.percent = progress.percent_done.clamp(0.0, 1.0);
                            running.bytes_done = progress.bytes_done;
                            running.bytes_total = progress.total_bytes;
                            running.files_done = progress.files_done;
                            running.files_total = progress.total_files;
                            running.seconds_remaining = progress.seconds_remaining;
                        }
                        self.notify();
                    },
                );
                match result {
                    Ok(summary) => break summary,
                    Err(engine::Error::Locked) if tries < 2 => {
                        // A restic that ended without removing its lock (the power cut).
                        let _ = self.shared.options.engine.unlock(&opened.repository);
                    }
                    Err(engine::Error::NoSpace) if tries < 4 => {
                        // Time Machine's rule: the oldest backups go to make room.
                        self.set_phase("cleaningUp");
                        let snapshots = self
                            .shared
                            .options
                            .engine
                            .snapshots(&opened.repository, Some(&config.computer))
                            .map_err(ended)?;
                        let Some(oldest) = policy::oldest_to_remove(&snapshots) else {
                            return Err(Ended::Failed("full"));
                        };
                        self.shared
                            .options
                            .engine
                            .forget_ids(&opened.repository, std::slice::from_ref(&oldest.id))
                            .map_err(ended)?;
                        self.shared
                            .options
                            .engine
                            .prune(&opened.repository, stop)
                            .map_err(ended)?;
                        self.set_phase("backingUp");
                    }
                    Err(engine::Error::NoSpace) => return Err(Ended::Failed("full")),
                    Err(error) => return Err(ended(error)),
                }
            }
        };
        let now = self.now();
        self.update(id, |record| {
            record.last_backup = Some(now);
            record.last_snapshot = Some(summary.snapshot_id.clone());
            record.last_failure = None;
            record.retry_at = None;
        });
        self.notify();

        // Thinning, removing what is unneeded and checking: the destination alone.
        let _exclusive = destination_lock.write().unwrap_or_else(|e| e.into_inner());
        let opened = self.repository(&destination).map_err(Ended::Failed)?;
        let engine = &self.shared.options.engine;
        self.set_phase("cleaningUp");
        let record = lock(&self.shared.state).records.get(id).cloned();
        match engine.forget(&opened.repository, &config.computer, &policy::keep()) {
            Ok(removed) => {
                if policy::prune_due(record.as_ref(), removed.len(), now) {
                    match engine.prune(&opened.repository, stop) {
                        Ok(()) => self.update(id, |r| r.last_prune = Some(now)),
                        Err(engine::Error::Stopped) => return Ok(()),
                        Err(e) => eprintln!("oxyos-backupd: removing unneeded data: {e}"),
                    }
                }
            }
            Err(e) => eprintln!("oxyos-backupd: thinning backups: {e}"),
        }
        if policy::check_due(record.as_ref(), now) {
            self.set_phase("checking");
            match engine.check(&opened.repository, Some(policy::CHECK_SUBSET), stop) {
                Ok(()) => self.update(id, |r| r.last_check = Some(now)),
                Err(engine::Error::Stopped) => return Ok(()),
                Err(e) => {
                    eprintln!("oxyos-backupd: checking backups: {e}");
                    self.update(id, |r| {
                        r.last_check = Some(now);
                        r.last_failure = Some(Failure {
                            reason: "damaged".into(),
                            at: now,
                        });
                    });
                }
            }
        }
        let snapshots = engine.snapshots(&opened.repository, Some(&config.computer));
        let stats = engine.stats(&opened.repository);
        let free = opened.mount.as_deref().and_then(disks::free_bytes);
        self.update(id, |record| {
            if let Ok(snapshots) = &snapshots {
                record.backups = snapshots.len() as u64;
                record.oldest_backup = snapshots.first().and_then(|s| s.seconds());
            }
            if let Ok(stats) = stats {
                record.used_bytes = Some(stats.total_size);
            }
            record.free_bytes = free;
        });
        Ok(())
    }

    // Destinations.

    fn destination_lock(&self, id: &str) -> Arc<RwLock<()>> {
        lock(&self.shared.locks)
            .entry(id.to_owned())
            .or_default()
            .clone()
    }

    /// A destination's repository, opened with the machine's key; a disk is mounted while the
    /// value lives.
    fn repository(&self, destination: &Destination) -> Result<Opened, &'static str> {
        let keys = self
            .shared
            .store
            .keys(&destination.id)
            .map_err(|_| "noKey")?
            .ok_or("noKey")?;
        let mut opened = self.location(
            destination,
            keys.share_password.as_deref().map(String::as_str),
        )?;
        opened.repository.password = keys.machine.clone();
        Ok(opened)
    }

    fn location(
        &self,
        destination: &Destination,
        share_password: Option<&str>,
    ) -> Result<Opened, &'static str> {
        match &destination.kind {
            Kind::Disk { uuid, .. } => {
                let disk = lock(&self.shared.state)
                    .connected
                    .get(uuid)
                    .cloned()
                    .ok_or("notConnected")?;
                let mount = self.acquire(&disk).map_err(|e| {
                    eprintln!("oxyos-backupd: mounting {}: {e}", disk.uuid);
                    "notConnected"
                })?;
                let path = mount.path.join(destination.repository_path());
                Ok(Opened {
                    repository: Repository {
                        location: Location::directory(&path),
                        password: Zeroizing::new(String::new()),
                    },
                    mount: Some(mount.path.clone()),
                    _mounted: Some(mount),
                })
            }
            Kind::Share(share) => {
                let obscured = share::obscure(
                    self.shared.options.engine.rclone(),
                    share_password.unwrap_or(""),
                )
                .map_err(|_| "failed")?;
                Ok(Opened {
                    repository: Repository {
                        location: share.location(&destination.repository_path(), &obscured),
                        password: Zeroizing::new(String::new()),
                    },
                    mount: None,
                    _mounted: None,
                })
            }
        }
    }

    fn acquire(&self, disk: &Disk) -> io::Result<Mounted> {
        let mut mounts = lock(&self.shared.mounts);
        let path = self.shared.options.disks.mount(disk)?;
        *mounts.entry(disk.uuid.clone()).or_default() += 1;
        Ok(Mounted {
            service: self.clone(),
            uuid: disk.uuid.clone(),
            path,
        })
    }

    fn release(&self, uuid: &str) {
        let mut mounts = lock(&self.shared.mounts);
        if let Some(count) = mounts.get_mut(uuid) {
            *count -= 1;
            if *count == 0 {
                mounts.remove(uuid);
                self.shared.options.disks.unmount(uuid);
            }
        }
    }

    // The state.

    /// The state, as a peer that is (or is not) an administrator is told it.
    pub fn status(&self, administrator: bool) -> Status {
        let now = self.now();
        let deadline = self.deadline(now);
        let state = lock(&self.shared.state);
        let destinations: Vec<DestinationStatus> = state
            .config
            .destinations
            .iter()
            .map(|d| {
                let record = state.records.get(&d.id).cloned().unwrap_or_default();
                let (kind, url) = match &d.kind {
                    Kind::Disk { .. } => ("disk", None),
                    Kind::Share(share) => ("share", Some(share.url())),
                };
                DestinationStatus {
                    id: d.id.clone(),
                    name: d.name.clone(),
                    kind,
                    url,
                    available: Self::available(&state, d),
                    last_backup: record.last_backup,
                    oldest_backup: record.oldest_backup,
                    backups: record.backups,
                    used_bytes: record.used_bytes,
                    free_bytes: record.free_bytes,
                    last_failure: record.last_failure,
                }
            })
            .collect();
        Status {
            automatic: state.config.automatic,
            last_backup: destinations.iter().filter_map(|d| d.last_backup).max(),
            next_backup: deadline,
            destinations,
            excluded: state.config.excluded.clone(),
            running: state.running.clone(),
            held: state.holds > 0,
            administrator,
        }
    }

    // Who asks.

    fn user_of(&self, uid: u32) -> Option<(String, PathBuf)> {
        let text = std::fs::read_to_string(&self.shared.options.passwd).ok()?;
        text.lines().find_map(|line| {
            let fields: Vec<&str> = line.split(':').collect();
            (fields.len() == 7 && fields[2].parse::<u32>().ok() == Some(uid))
                .then(|| (fields[0].to_owned(), PathBuf::from(fields[5])))
        })
    }

    /// Whether the peer may be served at all.
    fn served(&self, peer: &Peer) -> bool {
        !self.shared.options.development || peer.uid == rustix::process::getuid().as_raw()
    }

    /// Whether the peer is an administrator (root, or a member of `sudo`).
    pub fn administrator(&self, uid: u32) -> bool {
        if self.shared.options.development {
            return uid == rustix::process::getuid().as_raw();
        }
        if uid == 0 {
            return true;
        }
        let Some((name, _)) = self.user_of(uid) else {
            return false;
        };
        oxyos_account::users::group_members(&self.shared.options.group, "sudo")
            .ok()
            .flatten()
            .is_some_and(|members| members.contains(&name))
    }

    /// Whether `path` is one the peer may browse: in its own home (a developer's service: in a
    /// source), absolute and with no `.`, `..` or empty part.
    fn in_scope(&self, uid: u32, path: &str) -> bool {
        let path = Path::new(path);
        if !path.is_absolute()
            || path.as_os_str().len() > 4096
            || path.to_string_lossy().contains("//")
            || path
                .components()
                .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
        {
            return false;
        }
        if self.shared.options.development {
            return self.shared.options.sources.covers(path);
        }
        match self.user_of(uid) {
            Some((_, home)) if uid != 0 && home.is_absolute() && home != Path::new("/") => {
                path.starts_with(&home) && self.shared.options.sources.covers(path)
            }
            _ => false,
        }
    }

    // Conversations.

    /// Serves one connection until its peer closes it.
    pub fn serve(&self, connection: &Connection) -> io::Result<()> {
        let peer = connection.peer()?;
        let mut holding = false;
        let mut watching: Option<std::thread::JoinHandle<()>> = None;
        let mut watcher: Option<u64> = None;
        let result = loop {
            let request = match connection.receive::<Request>() {
                Ok(Some(Some(request))) => request,
                Ok(Some(None)) => {
                    connection.send(&Answer::refused("invalidRequest"))?;
                    continue;
                }
                Ok(None) => break Ok(()),
                Err(e) => break Err(e),
            };
            if !self.served(&peer) {
                connection.send(&Answer::refused("notPermitted"))?;
                continue;
            }
            match request {
                Request::Watch => {
                    if watching.is_none() {
                        let (sender, receiver) = mpsc::channel();
                        let number = self.shared.next_watcher.fetch_add(1, Ordering::SeqCst);
                        lock(&self.shared.watchers).push((number, sender));
                        watcher = Some(number);
                        let sending = connection.try_clone()?;
                        sending.set_send_timeout(std::time::Duration::from_secs(2))?;
                        let service = self.clone();
                        let administrator = self.administrator(peer.uid);
                        connection.send(&Answer::Status(self.status(administrator)))?;
                        watching = Some(std::thread::spawn(move || {
                            let mut last = None;
                            while receiver.recv().is_ok() {
                                while receiver.try_recv().is_ok() {}
                                let status = service.status(administrator);
                                if last.as_ref() == Some(&status) {
                                    continue;
                                }
                                if sending.send(&Answer::Status(status.clone())).is_err() {
                                    break;
                                }
                                last = Some(status);
                            }
                        }));
                    } else {
                        connection
                            .send(&Answer::Status(self.status(self.administrator(peer.uid))))?;
                    }
                }
                Request::Hold {
                    reason: HoldReason::LowPower,
                } => {
                    if !holding {
                        holding = true;
                        self.hold(true);
                    }
                    connection.send(&Answer::Held)?;
                }
                Request::Release {} => {
                    if holding {
                        holding = false;
                        self.hold(false);
                    }
                    connection.send(&Answer::Released)?;
                }
                Request::Retrieve {
                    destination,
                    snapshot,
                    path,
                } => self.retrieve(connection, peer.uid, &destination, &snapshot, &path)?,
                request => {
                    let answer = self.answer(peer.uid, request);
                    connection.send(&answer)?;
                }
            }
        };
        if holding {
            self.hold(false);
        }
        // Its channel goes, and with it its sending thread.
        if let Some(number) = watcher {
            lock(&self.shared.watchers).retain(|(n, _)| *n != number);
        }
        if let Some(watching) = watching {
            let _ = watching.join();
        }
        result
    }

    fn hold(&self, on: bool) {
        let mut state = lock(&self.shared.state);
        if on {
            state.holds += 1;
            // A scheduled backup stops (it resumes, deduplicated, once released); one asked for
            // goes on.
            if let (Some(running), Some(stop)) = (&state.running, &state.stop) {
                if !running.asked {
                    stop.stop();
                    state.stopped_for_hold = true;
                }
            }
        } else {
            state.holds = state.holds.saturating_sub(1);
        }
        drop(state);
        self.notify();
        self.wake();
    }

    /// Answers a request that needs no descriptor.
    pub fn answer(&self, uid: u32, request: Request) -> Answer {
        let administrator = self.administrator(uid);
        let admin_only = matches!(
            request,
            Request::Disks {}
                | Request::AddDisk { .. }
                | Request::AddShare { .. }
                | Request::Remove { .. }
                | Request::SetExcluded { .. }
                | Request::SetAutomatic { .. }
        );
        if admin_only && !administrator {
            return Answer::refused("notPermitted");
        }
        match request {
            Request::Status | Request::Watch => Answer::Status(self.status(administrator)),
            Request::BackUpNow {} => self.back_up_now(),
            Request::Stop {} => {
                let state = lock(&self.shared.state);
                match &state.stop {
                    Some(stop) => {
                        stop.stop();
                        Answer::Stopping
                    }
                    None => Answer::refused("notRunning"),
                }
            }
            Request::Disks {} => {
                self.refresh_disks();
                let state = lock(&self.shared.state);
                let added: BTreeSet<&str> = state
                    .config
                    .destinations
                    .iter()
                    .filter_map(|d| match &d.kind {
                        Kind::Disk { uuid, .. } => Some(uuid.as_str()),
                        Kind::Share(_) => None,
                    })
                    .collect();
                Answer::Disks(
                    state
                        .connected
                        .values()
                        .map(|disk| DiskChoice {
                            added: added.contains(disk.uuid.as_str()),
                            disk: disk.clone(),
                        })
                        .collect(),
                )
            }
            Request::AddDisk { uuid, password } => self.add_disk(&uuid, &password),
            Request::AddShare {
                share,
                share_password,
                password,
            } => self.add_share(share, share_password, &password),
            Request::Remove { id } => self.remove(&id),
            Request::SetExcluded { paths } => {
                if paths.len() > 1000
                    || paths
                        .iter()
                        .any(|p| !p.is_absolute() || !self.shared.options.sources.covers(p))
                {
                    return Answer::refused("invalidRequest");
                }
                self.configure(|config| config.excluded = paths)
            }
            Request::SetAutomatic { enabled } => {
                self.configure(|config| config.automatic = enabled)
            }
            Request::Versions { path } => self.versions(uid, &path),
            Request::Contents {
                destination,
                snapshot,
                path,
            } => self.contents(uid, &destination, &snapshot, &path),
            Request::Hold { .. } | Request::Release {} | Request::Retrieve { .. } => {
                Answer::refused("invalidRequest")
            }
        }
    }

    fn back_up_now(&self) -> Answer {
        let mut state = lock(&self.shared.state);
        if state.config.destinations.is_empty() {
            return Answer::refused("noDestination");
        }
        if state.running.is_some() || state.asked {
            return Answer::refused("running");
        }
        if !state
            .config
            .destinations
            .iter()
            .any(|d| Self::available(&state, d))
        {
            return Answer::refused("notConnected");
        }
        state.asked = true;
        drop(state);
        self.wake();
        Answer::Started
    }

    fn configure(&self, change: impl FnOnce(&mut Config)) -> Answer {
        let mut state = lock(&self.shared.state);
        let mut config = state.config.clone();
        change(&mut config);
        if self.shared.store.put_config(&config).is_err() {
            return Answer::refused("failed");
        }
        state.config = config;
        drop(state);
        self.notify();
        self.wake();
        Answer::Set
    }

    fn add_disk(&self, uuid: &str, password: &str) -> Answer {
        if password.chars().count() < MIN_PASSWORD {
            return Answer::refused("passwordTooShort");
        }
        self.refresh_disks();
        let disk = {
            let state = lock(&self.shared.state);
            if state
                .config
                .destinations
                .iter()
                .any(|d| matches!(&d.kind, Kind::Disk { uuid: u, .. } if u == uuid))
            {
                return Answer::refused("alreadyAdded");
            }
            match state.connected.get(uuid) {
                Some(disk) => disk.clone(),
                None => return Answer::refused("notConnected"),
            }
        };
        let name = if disk.label.is_empty() {
            "Backup Disk".to_owned()
        } else {
            disk.label.clone()
        };
        let kind = Kind::Disk {
            uuid: disk.uuid.clone(),
            fs_type: disk.fs_type.clone(),
        };
        self.add(name, kind, None, password)
    }

    fn add_share(
        &self,
        share: share::Share,
        share_password: Zeroizing<String>,
        password: &str,
    ) -> Answer {
        if let Err(reason) = share.validate() {
            return Answer::refused(reason);
        }
        if password.chars().count() < MIN_PASSWORD {
            return Answer::refused("passwordTooShort");
        }
        {
            let state = lock(&self.shared.state);
            if state
                .config
                .destinations
                .iter()
                .any(|d| matches!(&d.kind, Kind::Share(s) if s.host == share.host && s.share == share.share))
            {
                return Answer::refused("alreadyAdded");
            }
        }
        self.add(
            share.url(),
            Kind::Share(share),
            Some(share_password),
            password,
        )
    }

    /// Makes a destination's repository (its own folder of the destination's `OxyOS Backups`),
    /// opened by a machine key kept here and by the person's backup password, then keeps it.
    fn add(
        &self,
        name: String,
        kind: Kind,
        share_password: Option<Zeroizing<String>>,
        password: &str,
    ) -> Answer {
        let (Ok(id), Ok(folder), Ok(keys)) = (
            store::new_id(),
            store::new_id(),
            Keys::generate(share_password.clone()),
        ) else {
            return Answer::refused("failed");
        };
        let destination = Destination {
            id: id.clone(),
            name,
            kind,
            folder,
        };
        let opened =
            match self.location(&destination, share_password.as_deref().map(String::as_str)) {
                Ok(opened) => opened,
                Err(reason) => return Answer::refused(reason),
            };
        if let Some(mount) = &opened.mount {
            if let Err(e) = std::fs::create_dir_all(mount.join(store::BACKUPS_FOLDER)) {
                eprintln!("oxyos-backupd: making the backups' folder: {e}");
                return Answer::refused(if e.raw_os_error() == Some(28) {
                    "full"
                } else {
                    "failed"
                });
            }
        }
        let mut repository = opened.repository.clone();
        repository.password = keys.machine.clone();
        if let Err(error) = self.shared.options.engine.init(&repository, Some(password)) {
            eprintln!("oxyos-backupd: making the repository: {error}");
            return Answer::refused(reason(&error, &destination));
        }
        drop(opened);
        if self.shared.store.put_keys(&id, &keys).is_err() {
            return Answer::refused("failed");
        }
        let answer = self.configure(|config| config.destinations.push(destination));
        if !matches!(answer, Answer::Set) {
            let _ = self.shared.store.remove_keys(&id);
            return answer;
        }
        Answer::Added { id }
    }

    fn remove(&self, id: &str) -> Answer {
        {
            let state = lock(&self.shared.state);
            if !state.config.destinations.iter().any(|d| d.id == id) {
                return Answer::refused("notFound");
            }
            if state.running.as_ref().is_some_and(|r| r.destination == id) {
                if let Some(stop) = &state.stop {
                    stop.stop();
                }
            }
        }
        // Its backups stay on the destination, as Time Machine leaves them.
        let answer = self.configure(|config| config.destinations.retain(|d| d.id != id));
        if matches!(answer, Answer::Set) {
            let _ = self.shared.store.remove_keys(id);
            let mut state = lock(&self.shared.state);
            state.records.remove(id);
            let _ = self.shared.store.put_records(&state.records);
            return Answer::Removed;
        }
        answer
    }

    // Browsing and retrieving.

    fn destinations_available(&self) -> Vec<Destination> {
        let state = lock(&self.shared.state);
        state
            .config
            .destinations
            .iter()
            .filter(|d| Self::available(&state, d))
            .cloned()
            .collect()
    }

    fn computer(&self) -> String {
        lock(&self.shared.state).config.computer.clone()
    }

    fn versions(&self, uid: u32, path: &str) -> Answer {
        if !self.in_scope(uid, path) {
            return Answer::refused("notFound");
        }
        let computer = self.computer();
        let mut versions = Vec::new();
        for destination in self.destinations_available() {
            let destination_lock = self.destination_lock(&destination.id);
            let _shared = destination_lock.read().unwrap_or_else(|e| e.into_inner());
            let Ok(opened) = self.repository(&destination) else {
                continue;
            };
            let engine = &self.shared.options.engine;
            let Ok(snapshots) = engine.snapshots(&opened.repository, Some(&computer)) else {
                continue;
            };
            let times: BTreeMap<&str, i64> = snapshots
                .iter()
                .filter_map(|s| Some((s.id.as_str(), s.seconds()?)))
                .collect();
            let Ok(found) = engine.find(&opened.repository, &computer, path) else {
                continue;
            };
            for found in found {
                let Some(time) = times.get(found.snapshot.as_str()) else {
                    continue;
                };
                versions.push(Version {
                    destination: destination.id.clone(),
                    snapshot: found.snapshot,
                    time: *time,
                    kind: ItemKind::of(&found.node.kind),
                    size: found.node.size,
                    modified: found.node.mtime.as_deref().and_then(engine::seconds),
                });
            }
        }
        Answer::Versions(distinct(versions))
    }

    fn contents(&self, uid: u32, destination: &str, snapshot: &str, path: &str) -> Answer {
        if !self.in_scope(uid, path) || !valid_snapshot(snapshot) {
            return Answer::refused("notFound");
        }
        let Some(destination) = self
            .destinations_available()
            .into_iter()
            .find(|d| d.id == destination)
        else {
            return Answer::refused("notConnected");
        };
        let destination_lock = self.destination_lock(&destination.id);
        let _shared = destination_lock.read().unwrap_or_else(|e| e.into_inner());
        let opened = match self.repository(&destination) {
            Ok(opened) => opened,
            Err(reason) => return Answer::refused(reason),
        };
        match self
            .shared
            .options
            .engine
            .ls(&opened.repository, snapshot, path)
        {
            Ok(nodes) => Answer::Contents(
                nodes
                    .into_iter()
                    .filter(|n| {
                        n.path != path && Path::new(&n.path).parent() == Some(Path::new(path))
                    })
                    .map(|n| Item {
                        name: Path::new(&n.path)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        kind: ItemKind::of(&n.kind),
                        size: n.size,
                        modified: n.mtime.as_deref().and_then(engine::seconds),
                        path: n.path,
                    })
                    .collect(),
            ),
            Err(error) => Answer::refused(reason(&error, &destination)),
        }
    }

    fn retrieve(
        &self,
        connection: &Connection,
        uid: u32,
        destination: &str,
        snapshot: &str,
        path: &str,
    ) -> io::Result<()> {
        if !self.in_scope(uid, path) || !valid_snapshot(snapshot) {
            return connection.send(&Answer::refused("notFound"));
        }
        let Some(destination) = self
            .destinations_available()
            .into_iter()
            .find(|d| d.id == destination)
        else {
            return connection.send(&Answer::refused("notConnected"));
        };
        let destination_lock = self.destination_lock(&destination.id);
        let _shared = destination_lock.read().unwrap_or_else(|e| e.into_inner());
        let opened = match self.repository(&destination) {
            Ok(opened) => opened,
            Err(reason) => return connection.send(&Answer::refused(reason)),
        };
        let engine = &self.shared.options.engine;
        let node = match engine.ls(&opened.repository, snapshot, path) {
            Ok(nodes) => nodes.into_iter().find(|n| n.path == path),
            Err(error) => return connection.send(&Answer::refused(reason(&error, &destination))),
        };
        let Some(node) = node else {
            return connection.send(&Answer::refused("notFound"));
        };
        let kind = ItemKind::of(&node.kind);
        if !matches!(kind, ItemKind::File | ItemKind::Folder) {
            return connection.send(&Answer::refused("notFound"));
        }
        let dump = match engine.dump(&opened.repository, snapshot, path, kind == ItemKind::Folder) {
            Ok(dump) => dump,
            Err(error) => return connection.send(&Answer::refused(reason(&error, &destination))),
        };
        let (output, end) = dump.into_parts();
        let output = OwnedFd::from(output);
        let sent = connection.send_with_fd(
            &Answer::Retrieving {
                kind,
                size: if kind == ItemKind::File {
                    node.size
                } else {
                    None
                },
            },
            output.as_fd(),
        );
        drop(output);
        let finished = end.finish();
        sent?;
        match finished {
            Ok(()) => connection.send(&Answer::Retrieved),
            Err(error) => connection.send(&Answer::refused(reason(&error, &destination))),
        }
    }
}

/// A repository opened, and the disk it is on mounted while it lives.
struct Opened {
    repository: Repository,
    mount: Option<PathBuf>,
    _mounted: Option<Mounted>,
}

struct Mounted {
    service: Service,
    uuid: String,
    path: PathBuf,
}

impl Drop for Mounted {
    fn drop(&mut self) {
        self.service.release(&self.uuid);
    }
}

/// The reason the person is told for what restic said.
fn reason(error: &engine::Error, destination: &Destination) -> &'static str {
    match error {
        engine::Error::NoRepository => "noBackup",
        engine::Error::WrongPassword => "wrongPassword",
        engine::Error::Locked => "inUse",
        engine::Error::NoSpace => "full",
        engine::Error::Refused => "shareRefused",
        engine::Error::Unreachable => "unreachable",
        engine::Error::Stopped => "stopped",
        engine::Error::Program(_) | engine::Error::Incomplete => "failed",
        // A share that failed for a reason restic does not name: its computer, as a person sees it.
        engine::Error::Failed(_) if matches!(destination.kind, Kind::Share(_)) => "unreachable",
        engine::Error::Failed(_) => "failed",
    }
}

/// A snapshot's id as restic writes it: 8 to 64 hexadecimal digits.
fn valid_snapshot(id: &str) -> bool {
    (8..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The versions newest first, a run of backups holding the same item (its kind, size and time
/// of change) shown once, by its newest backup.
fn distinct(mut versions: Vec<Version>) -> Vec<Version> {
    versions.sort_by(|a, b| b.time.cmp(&a.time));
    let mut seen = BTreeSet::new();
    versions.retain(|v| seen.insert((v.kind as u8, v.size, v.modified)));
    versions
}

/// Sets the timer to `deadline` on the wall clock, or disarms it.
fn arm(timer: &OwnedFd, deadline: Option<i64>) {
    use rustix::time::{Itimerspec, TimerfdTimerFlags, Timespec};
    let value = match deadline {
        // A deadline already past fires at once.
        Some(deadline) => Timespec {
            tv_sec: deadline.max(1),
            tv_nsec: 0,
        },
        None => Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        },
    };
    let _ = rustix::time::timerfd_settime(
        timer,
        TimerfdTimerFlags::ABSTIME | TimerfdTimerFlags::CANCEL_ON_SET,
        &Itimerspec {
            it_interval: Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
            it_value: value,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(time: i64, size: u64, modified: i64) -> Version {
        Version {
            destination: "d".into(),
            snapshot: format!("{time:08x}"),
            time,
            kind: ItemKind::File,
            size: Some(size),
            modified: Some(modified),
        }
    }

    #[test]
    fn versions_are_each_change_once_newest_first() {
        let versions = distinct(vec![
            version(100, 1, 10),
            version(300, 2, 20),
            version(200, 1, 10),
            version(400, 2, 20),
        ]);
        let times: Vec<i64> = versions.iter().map(|v| v.time).collect();
        assert_eq!(times, vec![400, 200]);
    }

    #[test]
    fn snapshot_ids_are_restics() {
        assert!(valid_snapshot("30742fd0"));
        assert!(!valid_snapshot("latest"));
        assert!(!valid_snapshot("30742fd0/.."));
    }
}
