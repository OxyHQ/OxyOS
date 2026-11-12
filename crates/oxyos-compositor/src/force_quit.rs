//! Force Quit Applications (docs/adr/0067-hangs-and-force-quit.mdx), as a Mac's ⌥⌘⎋: the program
//! running the compositor (the session, and the development compositor alike) shows it, ends what
//! the person chooses, and asks a hung application for its report before it ends it.
//!
//! - **Asked for** by Super+Alt+Escape (the compositor's own key, `take_force_quit_shortcut`, so it
//!   works whatever an application or the shell is doing) or by the shell's Oxy menu
//!   (`{"forceQuit": {}}`, its `shell` grant, citing the person's input). Never while locked.
//! - **The sheet** is the session's system sheet `{"kind": "force-quit", "applications": [{"id",
//!   "name", "responding"}], "chosen": N}`, drawn by the shell: the applications the session
//!   launched, in their launch order, once each, those that do not answer marked; the frontmost
//!   chosen. The shell answers with `{"forceQuitApplication": {"sheet": N, "id": ID}}` (its grant,
//!   citing the press or key, only for the sheet shown), and the sheet goes; refused, it goes.
//! - **Ending one**: every process the session started for it, by its launch records. One that
//!   answers is ended at once (SIGKILL), as a Mac's Force Quit. One that does not (a Ping
//!   unanswered past the wait cursor's delay) is first asked for its hang's report
//!   (`Launcher::end_hung`: SIGQUIT, on which the Bloom host writes its JavaScript thread's stack
//!   into its report file), then killed [`HANG_REPORT_WAIT`] later on the program's clock, never
//!   blocking its turn, whatever it did with the request (one that ignores SIGQUIT ends all the
//!   same; the launcher starts every application with its signals at their defaults, so none
//!   inherits the session's ignoring it). Its report is written as a crash's, saying the hang ([`ForceQuit::exited`],
//!   `Crashes::note_hang`), and the person is told nothing more: they asked.
//! - **The dock's Force Quit** (docs/adr/0034-the-dock.mdx) ends the same way.

use crate::crashes::Crashes;
use crate::Compositor;
use oxyos_apps::{Exited, Launcher};
use oxyos_compositor_core::scene::SystemRequest;
use oxyos_compositor_core::ClientId;
use serde_json::json;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// How long a hung Bloom host has to write its stack once asked, before it is killed.
pub const HANG_REPORT_WAIT: Duration = Duration::from_millis(500);

/// A process asked for its hang's report.
#[derive(Debug)]
struct Hung {
    /// How long it had not answered when it was ended.
    unanswered: Duration,
    kill_at: Instant,
    killed: bool,
}

/// The part (see the module's documentation).
#[derive(Debug, Default)]
pub struct ForceQuit {
    /// The sheet shown and not answered, and the applications it listed.
    sheet: Option<(u32, Vec<String>)>,
    hung: BTreeMap<u32, Hung>,
}

/// One application of the sheet.
#[derive(Debug, PartialEq, Eq)]
struct Row {
    id: String,
    name: String,
    responding: bool,
}

fn ours(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text).is_ok_and(|asked| {
        asked.as_object().is_some_and(|asked| {
            asked.len() == 1
                && (asked.contains_key("forceQuit") || asked.contains_key("forceQuitApplication"))
        })
    })
}

impl ForceQuit {
    /// A turn at `now`: the shortcut and the shell's requests, the dock's force quits, and the
    /// hung processes whose time to write is up killed.
    pub fn serve(&mut self, compositor: &mut Compositor, launcher: &mut Launcher, now: Instant) {
        for pid in compositor.take_force_quits() {
            self.end(compositor, launcher, pid, now);
        }
        if compositor.scene.take_force_quit_shortcut() && !compositor.scene.is_locked() {
            self.show(compositor, launcher);
        }
        let requests = compositor
            .scene
            .take_system_requests_where(|request| ours(&request.text));
        for request in requests {
            self.handle(compositor, launcher, request, now);
        }
        if let Some((sheet, _)) = &self.sheet {
            if compositor.scene.take_sheet_answer(*sheet).is_some() {
                self.sheet = None;
            }
        }
        for (pid, hung) in self.hung.iter_mut() {
            if !hung.killed && now >= hung.kill_at {
                hung.killed = true;
                launcher.kill(*pid);
            }
        }
    }

    /// When the next hung process is to be killed, for the turn to wake then.
    pub fn deadline(&self) -> Option<Instant> {
        self.hung
            .values()
            .filter(|hung| !hung.killed)
            .map(|hung| hung.kill_at)
            .min()
    }

    /// The launcher's exits of a turn: each one ended as hung has its report written, saying how
    /// long it had not answered and what it reported of its JavaScript thread.
    pub fn exited(&mut self, crashes: &mut Crashes, exits: &[Exited]) {
        for exit in exits {
            if let Some(hung) = self.hung.remove(&exit.pid) {
                crashes.note_hang(exit, hung.unanswered);
            }
        }
    }

    /// The clients of the process `pid`.
    fn clients_of(compositor: &Compositor, pid: u32) -> Vec<ClientId> {
        compositor
            .peers
            .iter()
            .filter(|(_, peer)| u32::try_from(peer.pid).ok() == Some(pid))
            .map(|(client, _)| *client)
            .collect()
    }

    /// Ends the process `pid`: at once if it answers, else after its hang's report.
    fn end(&mut self, compositor: &Compositor, launcher: &mut Launcher, pid: u32, now: Instant) {
        let unanswered = Self::clients_of(compositor, pid)
            .into_iter()
            .filter(|client| compositor.scene.busy(*client))
            .filter_map(|client| compositor.scene.unanswered_for(client))
            .max();
        match unanswered {
            Some(unanswered) => self.end_unanswered(launcher, pid, unanswered, now),
            None => {
                if launcher.force_quit(pid) {
                    eprintln!("[force quit] {pid} ended");
                }
            }
        }
    }

    /// Ends the hung process `pid`, which has not answered for `unanswered`: asked for its report
    /// (SIGQUIT), then killed [`HANG_REPORT_WAIT`] later by [`ForceQuit::serve`] whatever it did
    /// with the request, so a program that ignores SIGQUIT, or blocks it, ends all the same; one
    /// that cannot even be asked is killed at once.
    fn end_unanswered(
        &mut self,
        launcher: &mut Launcher,
        pid: u32,
        unanswered: Duration,
        now: Instant,
    ) {
        if launcher.end_hung(pid) {
            eprintln!(
                "[force quit] {pid} did not answer for {} ms: its report, then the end",
                unanswered.as_millis()
            );
            self.hung.insert(
                pid,
                Hung {
                    unanswered,
                    kill_at: now + HANG_REPORT_WAIT,
                    killed: false,
                },
            );
        } else if launcher.force_quit(pid) {
            eprintln!(
                "[force quit] {pid} did not answer, and its report could not be asked: ended"
            );
        }
    }

    /// The applications the session launched, once each in launch order, whether each answers.
    fn rows(compositor: &Compositor, launcher: &Launcher) -> Vec<Row> {
        let mut rows: Vec<Row> = Vec::new();
        for (pid, launched) in launcher.running() {
            if launched.chooser {
                continue;
            }
            let busy = Self::clients_of(compositor, pid)
                .iter()
                .any(|client| !compositor.scene.responding(*client));
            match rows.iter_mut().find(|row| row.id == launched.id) {
                Some(row) => row.responding &= !busy,
                None => rows.push(Row {
                    id: launched.id.clone(),
                    name: launched.name.clone(),
                    responding: !busy,
                }),
            }
        }
        rows
    }

    /// Shows the sheet (one at a time: shown again, the old one goes).
    fn show(&mut self, compositor: &mut Compositor, launcher: &Launcher) -> Option<u32> {
        if let Some((old, _)) = self.sheet.take() {
            compositor.scene.dismiss_sheet(old);
        }
        let rows = Self::rows(compositor, launcher);
        let frontmost = compositor
            .scene
            .active_window()
            .and_then(|key| {
                compositor
                    .peers
                    .get(&key.client)
                    .and_then(|peer| u32::try_from(peer.pid).ok())
            })
            .and_then(|pid| launcher.application_of(pid))
            .map(|launched| launched.id.clone());
        let chosen = frontmost
            .and_then(|id| rows.iter().position(|row| row.id == id))
            .unwrap_or(0);
        let applications: Vec<serde_json::Value> = rows
            .iter()
            .map(|row| json!({ "id": row.id, "name": row.name, "responding": row.responding }))
            .collect();
        let text = json!({ "kind": "force-quit", "applications": applications, "chosen": chosen });
        let sheet = compositor.scene.show_sheet(None, &text.to_string())?;
        self.sheet = Some((sheet, rows.into_iter().map(|row| row.id).collect()));
        Some(sheet)
    }

    fn handle(
        &mut self,
        compositor: &mut Compositor,
        launcher: &mut Launcher,
        request: SystemRequest,
        now: Instant,
    ) {
        let refuse = |compositor: &mut Compositor, reason: &str| {
            compositor.scene.system_text(
                request.client,
                request.serial,
                &json!({ "refused": { "reason": reason } }).to_string(),
            );
        };
        let shell = compositor
            .scene
            .grants_of(request.client)
            .is_some_and(|grants| grants.shell);
        if !shell {
            return refuse(compositor, "notPermitted");
        }
        if !request.with_input {
            return refuse(compositor, "needsInput");
        }
        if compositor.scene.is_locked() {
            return refuse(compositor, "locked");
        }
        let asked: serde_json::Value = serde_json::from_str(&request.text).unwrap_or_default();
        if asked.get("forceQuit").is_some() {
            match self.show(compositor, launcher) {
                Some(sheet) => compositor.scene.system_text(
                    request.client,
                    request.serial,
                    &json!({ "shown": { "sheet": sheet } }).to_string(),
                ),
                None => refuse(compositor, "busy"),
            }
            return;
        }
        let chosen = &asked["forceQuitApplication"];
        let (Some(sheet), Some(id)) = (chosen["sheet"].as_u64(), chosen["id"].as_str()) else {
            return refuse(compositor, "invalidRequest");
        };
        let listed = self
            .sheet
            .as_ref()
            .is_some_and(|(shown, ids)| u64::from(*shown) == sheet && ids.iter().any(|i| i == id));
        if !listed {
            return refuse(compositor, "notFound");
        }
        let (shown, _) = self.sheet.take().expect("listed");
        compositor.scene.dismiss_sheet(shown);
        let pids: Vec<u32> = launcher
            .running()
            .filter(|(_, launched)| launched.id == id)
            .map(|(pid, _)| pid)
            .collect();
        for pid in pids {
            self.end(compositor, launcher, pid, now);
        }
        compositor.scene.system_text(
            request.client,
            request.serial,
            &json!({ "forceQuit": { "id": id } }).to_string(),
        );
    }
}

#[cfg(test)]
mod tests;
