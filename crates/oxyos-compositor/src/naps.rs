//! What the session does with an application by how the person sees it
//! (docs/adr/0066-quality-of-service.mdx, decisions 4 and 5): as a Mac's window server gives the
//! frontmost application urgency and naps what is not seen.
//!
//! - **Told** (decision 5): each client with an application window is told how the session
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
use oxyos_compositor_core::ClientId;
use oxyos_protocol::Visibility;
use oxyos_scopes::{CpuWeight, Properties, Scopes, Unit};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

/// How long an application stays hidden, silent and holding nothing back before it naps (a Mac
/// naps after a few seconds out of sight; Android freezes a cached application after ten).
pub const NAP_AFTER: Duration = Duration::from_secs(10);

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

/// The applications' states, what each client was told, and the scopes' weights.
pub struct Naps {
    scopes: Option<Scopes>,
    told: BTreeMap<ClientId, Visibility>,
    applications: BTreeMap<u32, Application>,
}

impl Naps {
    /// With `scopes`, the launched applications' scopes are weighed; without (no user manager,
    /// the development compositor without the benchmark's control group), clients are still told.
    pub fn new(scopes: Option<Scopes>) -> Self {
        Self {
            scopes,
            told: BTreeMap::new(),
            applications: BTreeMap::new(),
        }
    }

    /// Each turn: tells each application client how it is presented when that changed, and sets
    /// the weights of the applications `running` (each process the session launched, by pid, and
    /// its id) whose state changed. `background` are the pids running only in the background (ADR
    /// 0060), `busy` those audible or holding an inhibitor.
    pub fn serve(
        &mut self,
        compositor: &mut Compositor,
        running: &[(u32, String)],
        background: &BTreeSet<u32>,
        busy: &BTreeSet<u32>,
        now: Instant,
    ) {
        let presences = compositor.scene.presences();
        // A client told before whose toplevels are all gone (closed, its process going on) is
        // hidden; one gone is forgotten.
        self.told
            .retain(|client, _| compositor.peers.contains_key(client));
        let mut by_client: Vec<(ClientId, Visibility)> = presences.into_iter().collect();
        for client in self.told.keys() {
            if !by_client.iter().any(|(told, _)| told == client) {
                by_client.push((*client, Visibility::Hidden));
            }
        }
        let mut by_pid: BTreeMap<u32, Visibility> = BTreeMap::new();
        for (client, state) in by_client {
            if self.told.get(&client) != Some(&state)
                && compositor.scene.tell_visibility(client, state)
            {
                self.told.insert(client, state);
            }
            if let Some(pid) = compositor
                .peers
                .get(&client)
                .and_then(|peer| u32::try_from(peer.pid).ok())
            {
                let best = by_pid.entry(pid).or_insert(state);
                *best = (*best).max(state);
            }
        }
        let changes = self.update(&by_pid, running, background, busy, now);
        if let Some(scopes) = &self.scopes {
            for (_, unit, properties) in changes {
                scopes.set(&unit, properties);
            }
        }
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
