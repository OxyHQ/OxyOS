//! What the session does with an application by how the person sees it
//! (docs/adr/0066-quality-of-service.mdx, decisions 4 and 5): as a Mac's window server gives the
//! frontmost application urgency and naps what is not seen.
//!
//! - **Told** (decision 5), by the scene itself every turn (`Scene::tell_presences`, whatever
//!   program runs the compositor): each client with an application window, how the session
//!   presents it, all its windows together (`AppVisibility`: focused, visible, hidden), as it
//!   connects with a window and whenever that changes. Hidden, its host stops what the display's
//!   refresh paces (requestAnimationFrame, Animated's and Reanimated's frames), says React Native's
//!   AppState is `background` (`inactive` while visible but not focused) and lengthens its timers'
//!   slack, until it is shown.
//! - **Weighed** (decision 4): each application the session launched has its scope's CPU weight
//!   set by its state, off the turn ([`oxyos_scopes::Scopes`]): focused 400, visible 100, hidden
//!   20, running only in the background (ADR 0060) the idle class. An application hidden but
//!   audible (a PipeWire stream of its process playing) or holding one of logind's inhibitors is
//!   doing something the person asked for: it keeps the visible weight and never naps.
//! - **Napped** (decision 5): hidden for [`NAP_AFTER`], neither audible nor inhibiting, it naps:
//!   its disk's weight drops too (`IOWeight` 10). Shown, focused, or audible again, it is woken at
//!   once; a message or input to a hidden application reaches its process whatever its weight.
//!
//! An application that has shown no window yet (being launched, a program with none) is left at
//! the default weight. Only weights change: on an idle machine everything runs as before.

use crate::Compositor;
use oxyos_protocol::Visibility;
use oxyos_scopes::{CpuWeight, Properties, Scopes, Unit};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

/// How long an application stays hidden, silent and holding nothing back before it naps (a Mac
/// naps after a few seconds out of sight; Android freezes a cached application after ten).
pub const NAP_AFTER: Duration = Duration::from_secs(10);
/// The most of the machine's memory the focused application's claim may protect, in percent: its
/// working set kept, as a Mac keeps the frontmost application's, but never so much that one
/// application holds back what everything else needs.
pub const CLAIM_CAP_PERCENT: u64 = 25;
/// How often the focused application's claim is measured again while it stays focused: on a turn
/// that happens anyway, so nothing wakes the session for it.
pub const CLAIM_EVERY: Duration = Duration::from_secs(10);
/// What a claim is rounded up to: a working set that grows or shrinks by a little changes no
/// claim, so neither the user manager nor the accounts' service is told every ten seconds.
pub const CLAIM_STEP: u64 = 16 << 20;

/// What each application claims of protected memory (`MemoryLow`) by its state, and their sum,
/// which is exactly what `app.slice` is given (decision 4): with the kernel's recursive
/// protection, protection a slice holds that no child claims flows to its children by their usage,
/// so a slice given more would protect a hog beside the focused application. Only the focused
/// application claims, its memory now (`usage`), at most `cap`; visible, hidden and background ones
/// claim nothing: the protection is for what the person is using, and an application focused
/// again claims at once.
pub fn claims(
    activities: &BTreeMap<u32, Activity>,
    usage: &BTreeMap<u32, u64>,
    cap: u64,
) -> (BTreeMap<u32, u64>, u64) {
    let claims: BTreeMap<u32, u64> = activities
        .iter()
        .filter(|(_, activity)| **activity == Activity::Focused)
        .filter_map(|(pid, _)| {
            let bytes = usage.get(pid)?.div_ceil(CLAIM_STEP) * CLAIM_STEP;
            Some((*pid, bytes.min(cap)))
        })
        .filter(|(_, bytes)| *bytes > 0)
        .collect();
    let sum = claims.values().sum::<u64>().min(cap);
    (claims, sum)
}

/// A process's control group's memory now (`memory.current`), as the kernel accounts it.
fn memory_current(pid: u32) -> Option<u64> {
    let cgroup = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
    let path = cgroup.lines().find_map(|line| line.strip_prefix("0::"))?;
    std::fs::read_to_string(format!("/sys/fs/cgroup{path}/memory.current"))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// The machine's memory, from `/proc/meminfo`.
fn memory_total() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kib: u64 = meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()?;
    Some(kib * 1024)
}

/// What the session makes of an application, by what it knows of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    /// No window shown yet, or none ever: left at the default weight.
    Unknown,
    Focused,
    Visible,
    /// Hidden, but audible or holding an inhibitor: as visible, never napped.
    Busy,
    /// Hidden since the instant given.
    Hidden(Instant),
    /// Hidden, silent and holding nothing back for `NAP_AFTER`.
    Napping,
    /// Running only in the background (ADR 0060): its windows closed, the person allowing it.
    Background,
}

impl Activity {
    /// The scope's properties in this state (decision 4's weights, decision 5's nap).
    pub fn properties(self) -> Properties {
        let (cpu, io) = match self {
            Activity::Focused => (CpuWeight::Weight(400), 100),
            Activity::Unknown | Activity::Visible | Activity::Busy => (CpuWeight::Weight(100), 100),
            Activity::Hidden(_) => (CpuWeight::Weight(20), 100),
            Activity::Napping => (CpuWeight::Weight(20), 10),
            Activity::Background => (CpuWeight::Idle, 10),
        };
        Properties {
            cpu_weight: Some(cpu),
            io_weight: Some(io),
            memory_low: None,
        }
    }

    /// The same state for the weights (a hidden one whenever it began).
    fn same(self, other: Activity) -> bool {
        match (self, other) {
            (Activity::Hidden(_), Activity::Hidden(_)) => true,
            (a, b) => a == b,
        }
    }
}

/// What an application is now, from what it was and what the session sees of it: its process's
/// presence (the most shown of its clients'), whether it runs only in the background, whether it
/// is audible or holds an inhibitor, and the time.
pub fn decide(
    before: Activity,
    presence: Option<Visibility>,
    background: bool,
    busy: bool,
    now: Instant,
) -> Activity {
    if background {
        return Activity::Background;
    }
    match presence {
        None => Activity::Unknown,
        Some(Visibility::Focused) => Activity::Focused,
        Some(Visibility::Visible) => Activity::Visible,
        Some(Visibility::Hidden) if busy => Activity::Busy,
        Some(Visibility::Hidden) => match before {
            Activity::Napping => Activity::Napping,
            Activity::Hidden(since) if now.saturating_duration_since(since) >= NAP_AFTER => {
                Activity::Napping
            }
            Activity::Hidden(since) => Activity::Hidden(since),
            _ => Activity::Hidden(now),
        },
    }
}

struct Application {
    id: String,
    activity: Activity,
    /// What its scope was last given; none, nothing yet (the default).
    applied: Option<Activity>,
}

/// The applications' states and their scopes' weights.
pub struct Naps {
    scopes: Option<Scopes>,
    applications: BTreeMap<u32, Application>,
    /// Each application's claim of protected memory as last set, and `app.slice`'s.
    claimed: BTreeMap<u32, u64>,
    slice_claim: Option<u64>,
    /// When the claims were last measured, and for which focused applications.
    claimed_at: Option<(Instant, Vec<u32>)>,
    /// The most one claim may be (`CLAIM_CAP_PERCENT` of the machine's memory).
    cap: u64,
    /// Told `app.slice`'s claim each time it changes ([`Naps::with_claim_listener`]).
    claim_listener: Option<Box<dyn FnMut(u64)>>,
}

impl Naps {
    /// With `scopes`, the launched applications' scopes are weighed; without (no user manager,
    /// the development compositor without the benchmark's control group), only their states are
    /// kept (the scene tells the clients either way).
    pub fn new(scopes: Option<Scopes>) -> Self {
        Self {
            scopes,
            applications: BTreeMap::new(),
            claimed: BTreeMap::new(),
            slice_claim: None,
            claimed_at: None,
            cap: memory_total().unwrap_or(0) * CLAIM_CAP_PERCENT / 100,
            claim_listener: None,
        }
    }

    /// `listener` is told `app.slice`'s claim (the sum of the applications' claims, in bytes) each
    /// time it changes: at most once a focus change or `CLAIM_EVERY`, and only by `CLAIM_STEP` or
    /// more. Its ancestors, system units only root sets, must protect at least as much for the
    /// claim to reach the focused application (decision 7): the session tells the accounts'
    /// service.
    pub fn with_claim_listener(mut self, listener: Box<dyn FnMut(u64)>) -> Self {
        self.claim_listener = Some(listener);
        self
    }

    /// Each turn: sets the weights of the applications `running` (each process the session
    /// launched, by pid, and its id) whose state changed. `background` are the pids running only in
    /// the background (ADR 0060), `busy` those audible or holding an inhibitor.
    pub fn serve(
        &mut self,
        compositor: &Compositor,
        running: &[(u32, String)],
        background: &BTreeSet<u32>,
        busy: &BTreeSet<u32>,
        now: Instant,
    ) {
        // What the scene told each application client this turn (Scene::tell_presences), by
        // process: the most shown of its clients'.
        let mut by_pid: BTreeMap<u32, Visibility> = BTreeMap::new();
        for (client, state) in compositor.scene.presented() {
            if let Some(pid) = compositor
                .peers
                .get(client)
                .and_then(|peer| u32::try_from(peer.pid).ok())
            {
                let best = by_pid.entry(pid).or_insert(*state);
                *best = (*best).max(*state);
            }
        }
        let changes = self.update(&by_pid, running, background, busy, now);
        if let Some(scopes) = &self.scopes {
            for (_, unit, properties) in changes {
                scopes.set(&unit, properties);
            }
        }
        self.claim(now);
    }

    /// The focused application's working set protected, and `app.slice` given exactly the sum of
    /// the claims (`claims`): measured again as the focus changes, as an application claiming
    /// ends, and every `CLAIM_EVERY` while one stays focused.
    fn claim(&mut self, now: Instant) {
        let Some(scopes) = &self.scopes else {
            return;
        };
        let activities: BTreeMap<u32, Activity> = self
            .applications
            .iter()
            .map(|(pid, application)| (*pid, application.activity))
            .collect();
        let focused: Vec<u32> = activities
            .iter()
            .filter(|(_, activity)| **activity == Activity::Focused)
            .map(|(pid, _)| *pid)
            .collect();
        let gone = self.claimed.keys().any(|pid| !activities.contains_key(pid));
        let due = gone
            || self.claimed_at.as_ref().is_none_or(|(at, was)| {
                *was != focused || now.saturating_duration_since(*at) >= CLAIM_EVERY
            });
        if !due {
            return;
        }
        let usage: BTreeMap<u32, u64> = focused
            .iter()
            .filter_map(|pid| Some((*pid, memory_current(*pid)?)))
            .collect();
        let (claims, sum) = claims(&activities, &usage, self.cap);
        let set = |pid: u32, bytes: u64| {
            if let Some(application) = self.applications.get(&pid) {
                scopes.set(
                    &Unit::application(&application.id, pid),
                    Properties {
                        memory_low: Some(bytes),
                        ..Properties::default()
                    },
                );
            }
        };
        for pid in activities.keys() {
            let want = claims.get(pid).copied().unwrap_or(0);
            if self.claimed.get(pid).copied().unwrap_or(0) != want {
                set(*pid, want);
            }
        }
        if self.slice_claim != Some(sum) {
            scopes.set(
                &Unit::slice(oxyos_scopes::Slice::App),
                Properties {
                    memory_low: Some(sum),
                    ..Properties::default()
                },
            );
            self.slice_claim = Some(sum);
            if let Some(listener) = self.claim_listener.as_mut() {
                listener(sum);
            }
        }
        self.claimed = claims;
        self.claimed_at = Some((now, focused));
    }

    /// The applications' new states, and for each whose weights change, its scope and what to set:
    /// [`Naps::serve`] without the scene, for the tests.
    pub fn update(
        &mut self,
        presences: &BTreeMap<u32, Visibility>,
        running: &[(u32, String)],
        background: &BTreeSet<u32>,
        busy: &BTreeSet<u32>,
        now: Instant,
    ) -> Vec<(u32, Unit, Properties)> {
        let mut changes = Vec::new();
        let gone: Vec<u32> = self
            .applications
            .keys()
            .filter(|pid| !running.iter().any(|(running, _)| running == *pid))
            .copied()
            .collect();
        for pid in gone {
            if let (Some(application), Some(scopes)) =
                (self.applications.remove(&pid), &self.scopes)
            {
                scopes.forget(&Unit::application(&application.id, pid));
            }
        }
        for (pid, id) in running {
            let application = self.applications.entry(*pid).or_insert(Application {
                id: id.clone(),
                activity: Activity::Unknown,
                applied: None,
            });
            application.activity = decide(
                application.activity,
                presences.get(pid).copied(),
                background.contains(pid),
                busy.contains(pid),
                now,
            );
            let activity = application.activity;
            let unchanged = application
                .applied
                .is_some_and(|applied| applied.same(activity));
            // Nothing set yet and nothing known: the default stands.
            if unchanged || (application.applied.is_none() && activity == Activity::Unknown) {
                continue;
            }
            application.applied = Some(activity);
            changes.push((
                *pid,
                Unit::application(&application.id, *pid),
                activity.properties(),
            ));
        }
        changes
    }

    /// When an application hidden now naps, if one will: the turn's deadline. None while none is
    /// waiting to (nothing wakes the session for this otherwise).
    pub fn deadline(&self) -> Option<Instant> {
        self.applications
            .values()
            .filter_map(|application| match application.activity {
                Activity::Hidden(since) => Some(since + NAP_AFTER),
                _ => None,
            })
            .min()
    }

    /// Each launched application's state, by pid (the development compositor reports them).
    pub fn activities(&self) -> impl Iterator<Item = (u32, &str, Activity)> {
        self.applications
            .iter()
            .map(|(pid, application)| (*pid, application.id.as_str(), application.activity))
    }
}

#[cfg(test)]
mod tests;
