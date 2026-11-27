//! The shell watched (docs/adr/0061-crashes-and-resetting-an-application.mdx, decision 6, amended
//! 10 November 2026), as a Mac's window server and launchd watch the Dock: a shell that stops
//! answering is ended and started again, so a hung shell is never a dead Dock, menu bar and lock
//! screen. One set of decisions for the session and the development compositor; each sends the
//! signals to its own shells.
//!
//! - **Pinged whatever the person does**: each shell process is sent a Ping every [`KEEP_ALIVE`]
//!   (one outstanding at a time, the scene's), not only when input reaches it, so a shell nobody
//!   touches is watched too: the lock screen most of all.
//! - **Armed on its first answer**: a shell is watched once it answered a Ping, so the time its
//!   JavaScript takes to start on a loaded machine is never a hang (its start has the session's own
//!   deadline).
//! - **Hung after [`SHELL_HANG`]** without an answer: asked for its report first ([`ShellSignal::Report`]:
//!   `SIGQUIT`, on which the Bloom host writes its JavaScript thread's stack into its report file,
//!   with `SIGCONT` for one stopped), then ended [`REPORT_WAIT`] later ([`ShellSignal::End`]:
//!   `SIGKILL`), never blocking the turn. The program starts it again as it starts a shell that
//!   crashed (at once the first time, then ever later), its report saying the hang
//!   ([`ShellWatch::ended`]). While locked the session stays under the covers until the new shell
//!   shows the lock screen: the person is never left locked out, nor shown what is under it.

use crate::Compositor;
use oxyos_compositor_core::{ClientId, Scene};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

/// How often each shell is pinged, whatever the person does.
pub const KEEP_ALIVE: Duration = Duration::from_secs(2);
/// A Ping unanswered this long: the shell is hung.
pub const SHELL_HANG: Duration = Duration::from_secs(8);
/// How long a hung shell has to write its report before it is ended.
pub const REPORT_WAIT: Duration = crate::force_quit::HANG_REPORT_WAIT;

/// What the program sends a shell process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellSignal {
    /// Its report: `SIGQUIT`, and `SIGCONT` for one stopped.
    Report,
    /// Its end: `SIGKILL`.
    End,
}

/// The part (see the module's documentation).
#[derive(Debug, Default)]
pub struct ShellWatch {
    /// When each shell process was last pinged.
    pinged: BTreeMap<u32, Instant>,
    /// Shell processes that answered a Ping: watched.
    armed: BTreeSet<u32>,
    /// Shells ended for a hang: when they are killed, how long they had not answered, and
    /// whether they were.
    ending: BTreeMap<u32, (Instant, Duration, bool)>,
}

impl ShellWatch {
    /// A turn at `now`, with the program's shell processes `shells` (by pid): each pinged when
    /// due, and the signals the program is to send.
    pub fn serve(
        &mut self,
        compositor: &mut Compositor,
        shells: &[u32],
        now: Instant,
    ) -> Vec<(u32, ShellSignal)> {
        let shells: Vec<(u32, Vec<ClientId>)> = shells
            .iter()
            .map(|pid| {
                let clients = compositor
                    .peers
                    .iter()
                    .filter(|(_, peer)| u32::try_from(peer.pid).ok() == Some(*pid))
                    .map(|(client, _)| *client)
                    .collect();
                (*pid, clients)
            })
            .collect();
        self.serve_scene(&mut compositor.scene, &shells, now)
    }

    /// The same, each shell process with its connections.
    pub fn serve_scene(
        &mut self,
        scene: &mut Scene,
        shells: &[(u32, Vec<ClientId>)],
        now: Instant,
    ) -> Vec<(u32, ShellSignal)> {
        let running = |pid: &u32| shells.iter().any(|(shell, _)| shell == pid);
        self.pinged.retain(|pid, _| running(pid));
        self.armed.retain(running);
        let mut signals = Vec::new();
        for (pid, clients) in shells {
            let pid = *pid;
            if let Some((kill_at, _, killed)) = self.ending.get_mut(&pid) {
                if !*killed && now >= *kill_at {
                    *killed = true;
                    signals.push((pid, ShellSignal::End));
                }
                continue;
            }
            if clients.is_empty() {
                continue;
            }
            let unanswered = clients
                .iter()
                .filter_map(|client| scene.unanswered_for(*client))
                .max();
            match unanswered {
                None if self.pinged.contains_key(&pid) => {
                    self.armed.insert(pid);
                }
                Some(unanswered) if unanswered >= SHELL_HANG && self.armed.contains(&pid) => {
                    eprintln!(
                        "[shell] {pid} did not answer for {} ms: its report, then its end",
                        unanswered.as_millis()
                    );
                    self.ending
                        .insert(pid, (now + REPORT_WAIT, unanswered, false));
                    signals.push((pid, ShellSignal::Report));
                    continue;
                }
                _ => {}
            }
            let due = self
                .pinged
                .get(&pid)
                .is_none_or(|at| now.saturating_duration_since(*at) >= KEEP_ALIVE);
            if due {
                for client in clients {
                    scene.ping_client(*client);
                }
                self.pinged.insert(pid, now);
            }
        }
        signals
    }

    /// When the turn must wake: the next Ping, or a hung shell's end.
    pub fn deadline(&self) -> Option<Instant> {
        let pings = self
            .pinged
            .iter()
            .filter(|(pid, _)| !self.ending.contains_key(pid))
            .map(|(_, at)| *at + KEEP_ALIVE);
        let ends = self
            .ending
            .values()
            .filter(|(_, _, killed)| !killed)
            .map(|(at, _, _)| *at);
        pings.chain(ends).min()
    }

    /// The shell `pid` ended: how long it had not answered, if this part ended it for a hang.
    pub fn ended(&mut self, pid: u32) -> Option<Duration> {
        self.pinged.remove(&pid);
        self.armed.remove(&pid);
        self.ending
            .remove(&pid)
            .map(|(_, unanswered, _)| unanswered)
    }
}

#[cfg(test)]
mod tests;
