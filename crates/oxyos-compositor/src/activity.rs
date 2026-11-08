//! Activity Monitor for the program running the compositor (docs/adr/0063-activity-monitor.mdx),
//! the session authority: what Activity Monitor (the `activity` grant) is told of the machine's
//! use and every process's, and what it may ask: a process's details, a process of the person's
//! quit, an application quit.
//!
//! - **Sampled only while seen.** A client follows a view (`activityFollow`); it is sent a sample
//!   at once and every [`EVERY`] while a window of its is shown (mapped, neither minimized nor
//!   hidden, on the desktop shown, the session unlocked and the displays on). Hidden, the thread
//!   that samples waits on its jobs alone, with no deadline: nothing is read, nothing is sent, and
//!   nothing wakes (docs/performance.mdx, *Idle*).
//! - **Off the compositor's turn.** Reading every process takes milliseconds; the thread does it
//!   and wakes the turn with what it read, as the power part's does.
//! - **One count of energy.** The Energy view's applications are sampled into the battery's meter
//!   (`Power::sample_energy`), whose twelve-hour average they are told, so Settings › Battery and
//!   Activity Monitor say the same.

use crate::power::Power;
use crate::Compositor;
use oxyos_activity::{inspect, quit, App, AppEnergy, Sample, Sampler, View};
use oxyos_compositor_core::scene::SystemRequest;
use oxyos_compositor_core::{ClientId, Scene};
use oxyos_power::energy::{self, Running};
use serde::Deserialize;
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

/// How often a shown follower is sampled, as a Mac's Activity Monitor's *Normally*.
pub const EVERY: Duration = Duration::from_secs(2);
/// The first sample after following begins or the window comes back: rates over this, so the
/// window is not left without its numbers for a whole interval.
pub const FIRST: Duration = Duration::from_millis(500);

/// What a client asks, read strictly.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
enum Ask {
    /// Follow a view, or stop (`null`).
    #[serde(rename = "activityFollow")]
    Follow { view: Option<View> },
    /// Quit one of the person's processes, citing an input.
    #[serde(rename = "activityQuit")]
    Quit { pid: u32, started: u64, force: bool },
    /// Quit an application the session launched, citing an input.
    #[serde(rename = "activityQuitApp")]
    QuitApp { id: String, force: bool },
    /// One process's details.
    #[serde(rename = "activityInspect")]
    Inspect { pid: u32, started: u64 },
}

const ASKS: [&str; 4] = [
    "activityFollow",
    "activityQuit",
    "activityQuitApp",
    "activityInspect",
];

fn ours(text: &str) -> bool {
    serde_json::from_str::<HashMap<String, serde::de::IgnoredAny>>(text)
        .is_ok_and(|asked| asked.len() == 1 && asked.keys().all(|key| ASKS.contains(&key.as_str())))
}

fn refused(reason: &str) -> String {
    json!({ "refused": { "reason": reason } }).to_string()
}

fn failed(reason: &str) -> String {
    json!({ "failed": { "reason": reason } }).to_string()
}

/// What the thread is asked.
enum Job {
    /// The followers shown now, each with its view: none, the thread waits on its jobs alone.
    Follow(Vec<(ClientId, View)>),
    /// The applications the session launched that run.
    Apps(Vec<Running>),
    Quit {
        client: ClientId,
        serial: u32,
        pid: u32,
        started: u64,
        force: bool,
    },
    Inspect {
        client: ClientId,
        serial: u32,
        pid: u32,
        started: u64,
    },
}

/// What the thread did.
enum Done {
    Sampled {
        clients: Vec<ClientId>,
        sample: Box<Sample>,
        /// The Energy view's: each application's use, for the battery's meter.
        energy: Option<(Vec<Running>, Vec<energy::Read>)>,
    },
    Answer {
        client: ClientId,
        serial: u32,
        text: String,
    },
}

#[derive(Clone)]
struct Back {
    done: mpsc::Sender<Done>,
    wake: Arc<UnixStream>,
}

impl Back {
    fn send(&self, done: Done) {
        if self.done.send(done).is_ok() {
            let _ = (&*self.wake).write(&[1]);
        }
    }
}

/// The thread's side.
struct Worker {
    sampler: Sampler,
    /// Processes beyond the applications' whose GPU is read: the session's own.
    gpu: Vec<u32>,
    protected: Vec<u32>,
    uid: u32,
    running: Vec<Running>,
    /// Each application's name and icon, as the registry lists it, looked for once.
    listed: HashMap<String, (Option<String>, Option<String>)>,
    /// The samples it took, for the tests and the development compositor's steps.
    samples: Arc<std::sync::atomic::AtomicU64>,
    /// Its thread's id, once it runs.
    tid: Arc<std::sync::atomic::AtomicU32>,
}

impl Worker {
    /// The applications running, each with its icon and, when the session knows none, its name
    /// as the registry lists them (looked for once per application).
    fn apps(&mut self) -> Vec<App> {
        if self
            .running
            .iter()
            .any(|r| !self.listed.contains_key(&r.id))
        {
            let listed = oxyos_apps::Listing::read(None).applications(&oxyos_apps::languages());
            for running in &self.running {
                let found = listed
                    .iter()
                    .find(|application| application.id == running.id);
                self.listed.insert(
                    running.id.clone(),
                    (
                        found.map(|application| application.name.clone()),
                        found
                            .and_then(|application| application.icon.as_ref())
                            .map(|icon| icon.to_string_lossy().into_owned()),
                    ),
                );
            }
        }
        self.running
            .iter()
            .map(|running| {
                let (name, icon) = self.listed.get(&running.id).cloned().unwrap_or_default();
                App {
                    id: running.id.clone(),
                    // A launch the session named is its name; the development compositor's own
                    // launch knows only its id.
                    name: match name {
                        Some(name) if running.name == running.id => name,
                        _ => running.name.clone(),
                    },
                    pid: running.pid,
                    icon,
                }
            })
            .collect()
    }

    /// A sample of each view the followers shown follow, sent to them.
    fn sample(&mut self, following: &[(ClientId, View)], back: &Back, send: bool) {
        let apps = self.apps();
        let mut views: Vec<View> = following.iter().map(|(_, view)| *view).collect();
        views.dedup();
        for view in views {
            let sample = self.sampler.sample(view, Instant::now(), &apps, &self.gpu);
            self.samples
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if !send {
                continue;
            }
            let energy = (view == View::Energy).then(|| {
                let reads = energy::read(&self.sampler.procfs().energy(), &self.running);
                (self.running.clone(), reads)
            });
            back.send(Done::Sampled {
                clients: following
                    .iter()
                    .filter(|(_, v)| *v == view)
                    .map(|(client, _)| *client)
                    .collect(),
                sample: Box::new(sample),
                energy,
            });
        }
    }

    fn run(mut self, jobs: mpsc::Receiver<Job>, back: Back) {
        // Which thread it is (`/proc/thread-self` names it), for what counts its wakeups.
        if let Some(tid) = std::fs::read_link("/proc/thread-self")
            .ok()
            .and_then(|path| path.file_name()?.to_str()?.parse().ok())
        {
            self.tid.store(tid, std::sync::atomic::Ordering::Relaxed);
        }
        let mut following: Vec<(ClientId, View)> = Vec::new();
        let mut next: Option<Instant> = None;
        loop {
            // Waits on its jobs alone while nothing is followed or shown: no deadline.
            let job = match next {
                Some(deadline) => {
                    match jobs.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                        Ok(job) => Some(job),
                        Err(mpsc::RecvTimeoutError::Timeout) => None,
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
                None => match jobs.recv() {
                    Ok(job) => Some(job),
                    Err(_) => return,
                },
            };
            match job {
                Some(Job::Follow(mut shown)) => {
                    shown.sort_by_key(|(client, view)| (*view as u8, *client));
                    if shown.is_empty() {
                        following.clear();
                        next = None;
                        continue;
                    }
                    let views = |list: &[(ClientId, View)]| {
                        let mut views: Vec<View> = list.iter().map(|(_, v)| *v).collect();
                        views.dedup();
                        views
                    };
                    // A view newly followed: a reading to measure from, and its first sample
                    // soon after. A follower added to a view already sampled waits its turn.
                    if following.is_empty() || views(&following) != views(&shown) {
                        self.sampler.reset();
                        self.sample(&shown, &back, false);
                        next = Some(Instant::now() + FIRST);
                    }
                    following = shown;
                }
                Some(Job::Apps(running)) => self.running = running,
                Some(Job::Quit {
                    client,
                    serial,
                    pid,
                    started,
                    force,
                }) => {
                    let text = match quit::quit(
                        self.sampler.procfs(),
                        pid,
                        started,
                        force,
                        self.uid,
                        &self.protected,
                    ) {
                        Ok(()) => json!({ "activityQuit": {} }).to_string(),
                        Err(failure) => failed(failure.name()),
                    };
                    back.send(Done::Answer {
                        client,
                        serial,
                        text,
                    });
                }
                Some(Job::Inspect {
                    client,
                    serial,
                    pid,
                    started,
                }) => {
                    let app = self.app_of(pid);
                    let text = match inspect::inspect(
                        self.sampler.procfs(),
                        pid,
                        started,
                        self.uid,
                        app,
                    ) {
                        Some(details) => json!({ "activityInspect": details }).to_string(),
                        None => failed("notFound"),
                    };
                    back.send(Done::Answer {
                        client,
                        serial,
                        text,
                    });
                }
                None => {
                    self.sample(&following, &back, true);
                    // The next on the interval's beat; one fallen behind waits a whole interval.
                    let now = Instant::now();
                    let mut deadline = next.unwrap_or(now) + EVERY;
                    if deadline <= now {
                        deadline = now + EVERY;
                    }
                    next = Some(deadline);
                }
            }
        }
    }

    /// The application `pid` is of: launched as it, or below one launched.
    fn app_of(&self, pid: u32) -> Option<String> {
        let mut buffer = String::new();
        let mut current = pid;
        for _ in 0..64 {
            if let Some(running) = self.running.iter().find(|r| r.pid == current) {
                return Some(running.id.clone());
            }
            let path = self.sampler.procfs().of(current, "stat");
            let text = std::fs::read_to_string(path).ok()?;
            buffer.clear();
            buffer.push_str(&text);
            let after = &buffer[buffer.rfind(')')? + 1..];
            current = after.split_whitespace().nth(1)?.parse().ok()?;
            if current <= 1 {
                return None;
            }
        }
        None
    }
}

/// A client following: the view it follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Follower {
    view: View,
}

pub struct Activity {
    jobs: mpsc::Sender<Job>,
    done: mpsc::Receiver<Done>,
    woken: UnixStream,
    thread: Option<std::thread::JoinHandle<()>>,
    followers: BTreeMap<ClientId, Follower>,
    /// What the thread was last told of the followers shown, and of the applications.
    told: Vec<(ClientId, View)>,
    running: Vec<Running>,
    samples: Arc<std::sync::atomic::AtomicU64>,
    tid: Arc<std::sync::atomic::AtomicU32>,
    sent: u64,
}

impl Activity {
    /// Activity Monitor's part over the system's `/proc`, the session being this process.
    pub fn new() -> std::io::Result<Self> {
        Self::over(Sampler::system(), std::process::id())
    }

    /// Its part over `sampler`, the session being `session` (its process, and those it runs under,
    /// never quit; its GPU read as an application's).
    pub fn over(sampler: Sampler, session: u32) -> std::io::Result<Self> {
        let (wake, woken) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        woken.set_nonblocking(true)?;
        let (done, received) = mpsc::channel();
        let back = Back {
            done,
            wake: Arc::new(wake),
        };
        let (jobs, taken) = mpsc::channel::<Job>();
        let samples = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let tid = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let worker = Worker {
            protected: quit::protected(sampler.procfs(), session),
            sampler,
            gpu: vec![session],
            uid: rustix::process::getuid().as_raw(),
            running: Vec::new(),
            listed: HashMap::new(),
            samples: samples.clone(),
            tid: tid.clone(),
        };
        let thread = std::thread::Builder::new()
            .name("activity".into())
            .spawn(move || worker.run(taken, back))?;
        Ok(Self {
            jobs,
            done: received,
            woken,
            thread: Some(thread),
            followers: BTreeMap::new(),
            told: Vec::new(),
            running: Vec::new(),
            samples,
            tid,
            sent: 0,
        })
    }

    /// What wakes the compositor's turn when the thread is done (`Compositor::wakers`).
    pub fn wake_fd(&self) -> std::io::Result<OwnedFd> {
        Ok(self.woken.try_clone()?.into())
    }

    /// Samples taken (each view's, with the readings measured from), and samples sent to
    /// followers: what the development compositor's steps say.
    pub fn counts(&self) -> (u64, u64) {
        (
            self.samples.load(std::sync::atomic::Ordering::Relaxed),
            self.sent,
        )
    }

    /// The sampling thread's id (0 before it runs): what counts its wakeups reads its
    /// `/proc/self/task/TID`.
    pub fn thread_id(&self) -> u32 {
        self.tid.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Whether any client follows, shown or not.
    pub fn followed(&self) -> bool {
        !self.followers.is_empty()
    }

    fn send(&self, job: Job) {
        let _ = self.jobs.send(job);
    }

    /// A turn: the requests that are this part's, the followers shown, and what the thread did.
    /// `running` are the applications the session launched that run; `displays_off` whether the
    /// displays are off; `power` the battery's part, whose meter counts the energy.
    pub fn serve(
        &mut self,
        compositor: &mut Compositor,
        running: &[Running],
        mut power: Option<&mut Power>,
    ) {
        let displays_off = compositor.displays_off();
        let mut buffer = [0u8; 64];
        while matches!((&self.woken).read(&mut buffer), Ok(read) if read > 0) {}
        if running != self.running.as_slice() {
            self.running = running.to_vec();
            self.send(Job::Apps(self.running.clone()));
        }
        let scene = &mut compositor.scene;
        for request in scene.take_system_requests_where(|request| ours(&request.text)) {
            self.handle(scene, request);
        }
        // A follower gone (its client disconnected) follows no more.
        self.followers
            .retain(|client, _| scene.grants_of(*client).is_some());
        let shown: Vec<(ClientId, View)> = if displays_off {
            Vec::new()
        } else {
            self.followers
                .iter()
                .filter(|(client, _)| scene.client_shown(**client))
                .map(|(client, follower)| (*client, follower.view))
                .collect()
        };
        if shown != self.told {
            self.told = shown.clone();
            self.send(Job::Follow(shown));
        }
        for done in self.done.try_iter().collect::<Vec<_>>() {
            match done {
                Done::Answer {
                    client,
                    serial,
                    text,
                } => compositor.scene.system_text(client, serial, &text),
                Done::Sampled {
                    clients,
                    sample,
                    energy,
                } => {
                    let mut sample = *sample;
                    if let (Some(energy), Some(listed)) = (energy, sample.energy.as_mut()) {
                        merge_energy(listed, power.as_deref_mut(), &energy.0, &energy.1);
                    }
                    let text = json!({ "activity": sample }).to_string();
                    for client in clients {
                        // Only to who still follows that view and is still shown; as one file,
                        // since a sample is tens of kilobytes (or in chunks, to a client of an
                        // older protocol).
                        if self.told.contains(&(client, sample.view)) {
                            if !compositor.system_data(client, 0, &text) {
                                compositor.scene.system_text(client, 0, &text);
                            }
                            self.sent += 1;
                        }
                    }
                }
            }
        }
    }

    fn handle(&mut self, scene: &mut Scene, request: SystemRequest) {
        let (client, serial) = (request.client, request.serial);
        let Ok(ask) = serde_json::from_str::<Ask>(&request.text) else {
            scene.system_text(client, serial, &refused("invalidRequest"));
            return;
        };
        let Some(grants) = scene.grants_of(client) else {
            return;
        };
        if !grants.activity {
            scene.system_text(client, serial, &refused("notPermitted"));
            return;
        }
        match ask {
            Ask::Follow { view } => {
                match view {
                    Some(view) => {
                        self.followers.insert(client, Follower { view });
                    }
                    None => {
                        self.followers.remove(&client);
                    }
                }
                scene.system_text(client, serial, &json!({ "activityFollow": {} }).to_string());
            }
            Ask::Quit {
                pid,
                started,
                force,
            } => {
                if !request.with_input {
                    scene.system_text(client, serial, &refused("needsInput"));
                    return;
                }
                self.send(Job::Quit {
                    client,
                    serial,
                    pid,
                    started,
                    force,
                });
            }
            Ask::QuitApp { id, force } => {
                if !request.with_input {
                    scene.system_text(client, serial, &refused("needsInput"));
                    return;
                }
                let done = if force {
                    scene.force_quit_application_by_id(&id).unwrap_or(false)
                } else {
                    scene.quit_application_by_id(&id)
                };
                let text = if done {
                    json!({ "activityQuitApp": {} }).to_string()
                } else {
                    failed("notFound")
                };
                scene.system_text(client, serial, &text);
            }
            Ask::Inspect { pid, started } => self.send(Job::Inspect {
                client,
                serial,
                pid,
                started,
            }),
        }
    }
}

/// The Energy view's applications: their use sampled into the battery's meter, each told its
/// twelve-hour average, and those this session ran that no longer run, as the meter keeps them.
/// Without the battery's part (the login screen, a development compositor without one) the
/// applications running alone, with no average.
fn merge_energy(
    listed: &mut Vec<AppEnergy>,
    power: Option<&mut Power>,
    running: &[Running],
    reads: &[energy::Read],
) {
    let Some(power) = power else {
        return;
    };
    for app in power.sample_energy(running, reads) {
        match listed.iter_mut().find(|listed| listed.id == app.id) {
            Some(listed) => listed.average = Some(app.average * 100.0),
            None => listed.push(AppEnergy {
                id: app.id,
                name: app.name,
                impact: None,
                average: Some(app.average * 100.0),
                running: app.running,
            }),
        }
    }
}

impl Drop for Activity {
    fn drop(&mut self) {
        // The worker ends once its jobs' channel closes.
        let (closed, _) = mpsc::channel();
        self.jobs = closed;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests;
