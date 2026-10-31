//! What the session's shell is told of windows and workspaces, and what it may do to them
//! (ADR 0009). A window, a mapped toplevel, is known to the shell by a number the compositor gives
//! it, with its workspace, state, title and application id; each shell client is told what
//! changed since it was last told, so one that connects late is told everything. So are the
//! outputs, each with its place, scale and whether it is the primary (ADR 0018).

use super::{ClientId, Output, OutputId, Request, Role, Scene, SurfaceKey};
use oxyos_protocol::{ErrorCode, Event, PreeditText, Size, TextChunk};
use std::collections::BTreeMap;

/// A window's title, in bytes.
pub const MAX_TITLE_BYTES: usize = 256;
/// A window's application id, in bytes.
pub const MAX_APP_ID_BYTES: usize = 128;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Names {
    title: String,
    app_id: String,
}

/// A window as the shell sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Window {
    workspace: u32,
    minimized: bool,
    focused: bool,
    /// Its process is one the session started: its application id is the registry's.
    launched: bool,
    names: Names,
    /// Its size, logical pixels: the proportions the shell shows it in (ADR 0022).
    size: Size,
    /// How recently it was active, and whether it is hidden with its application (ADR 0024).
    used: u32,
    hidden: bool,
    /// Its process answers the compositor's pings (ADR 0034).
    responding: bool,
}

#[derive(Debug, Default)]
struct Told {
    windows: BTreeMap<u32, Window>,
    workspaces: Option<(u32, u32)>,
    /// Each output, and whether it was the primary.
    outputs: BTreeMap<OutputId, (Output, bool)>,
}

#[derive(Debug, Default)]
pub(super) struct ShellWindows {
    numbers: BTreeMap<SurfaceKey, u32>,
    next: u32,
    names: BTreeMap<SurfaceKey, Names>,
    /// A title or application id (`true`) being received: its text so far and the next index.
    receiving: BTreeMap<(SurfaceKey, bool), (String, u8)>,
    told: BTreeMap<ClientId, Told>,
    /// The application of each client whose process the session started (ADR 0013): its
    /// windows' id, whatever they claim.
    applications: BTreeMap<ClientId, String>,
}

impl Scene {
    /// A toplevel was made: the number the shell will know it by.
    pub(super) fn number_window(&mut self, key: SurfaceKey) {
        let shell = &mut self.shell_windows;
        shell.next += 1;
        shell.numbers.insert(key, shell.next);
    }

    pub(super) fn forget_window(&mut self, key: SurfaceKey) {
        let shell = &mut self.shell_windows;
        shell.numbers.remove(&key);
        shell.names.remove(&key);
        shell.receiving.retain(|(window, _), _| *window != key);
    }

    pub(super) fn forget_shell_client(&mut self, client: ClientId) {
        self.shell_windows.told.remove(&client);
        self.shell_windows.applications.remove(&client);
    }

    /// `client`'s process is one the session started for the application `id` (its launch
    /// record, ADR 0013): its windows are that application's, whatever they claim, and the
    /// shell is told so.
    pub fn set_application(&mut self, client: ClientId, id: &str) {
        if self.clients.contains_key(&client) {
            self.shell_windows
                .applications
                .insert(client, id.to_string());
            // Started for a notification's response (ADR 0026): it is given it now.
            self.deliver_pending_responses(client, id);
        }
    }

    /// The application `client`'s launch record names, if the session started it.
    pub fn application(&self, client: ClientId) -> Option<&str> {
        self.shell_windows
            .applications
            .get(&client)
            .map(String::as_str)
    }

    /// What makes windows one application, as the shell sees them: a launched process's
    /// registry id, else the id the window claims, else (it claims none) its client.
    pub(super) fn application_identity(&self, key: SurfaceKey) -> (bool, String, Option<ClientId>) {
        if let Some(id) = self.shell_windows.applications.get(&key.client) {
            return (true, id.clone(), None);
        }
        match self
            .shell_windows
            .names
            .get(&key)
            .map(|names| names.app_id.as_str())
        {
            Some(id) if !id.is_empty() => (false, id.to_string(), None),
            _ => (false, String::new(), Some(key.client)),
        }
    }

    /// The clients showing a window: a toplevel of theirs mapped (docs/adr/0030-installing-
    /// applications.mdx, decision 10: an installed version that showed one is kept).
    pub fn clients_showing_windows(&self) -> std::collections::BTreeSet<ClientId> {
        self.shell_windows
            .numbers
            .keys()
            .filter(|key| self.live(**key).is_some_and(|node| node.mapped))
            .map(|key| key.client)
            .collect()
    }

    /// The number the shell knows a window by.
    pub fn window_number(&self, key: SurfaceKey) -> Option<u32> {
        self.shell_windows.numbers.get(&key).copied()
    }

    pub(super) fn window_by_number(&self, number: u32) -> Option<SurfaceKey> {
        let found = self
            .shell_windows
            .numbers
            .iter()
            .find(|(_, n)| **n == number);
        found
            .map(|(key, _)| *key)
            .filter(|key| self.live(*key).is_some())
    }

    /// A chunk of a window's title or application id (SetTitle, SetAppId).
    pub(super) fn name_window(
        &mut self,
        key: SurfaceKey,
        app_id: bool,
        index: u8,
        last: bool,
        text: PreeditText,
    ) -> Result<(), ErrorCode> {
        let node = self.live(key).ok_or(ErrorCode::UnknownSurface)?;
        if node.role != Role::Toplevel {
            return Err(ErrorCode::InvalidRole);
        }
        let limit = if app_id {
            MAX_APP_ID_BYTES
        } else {
            MAX_TITLE_BYTES
        };
        let receiving = &mut self.shell_windows.receiving;
        let entry = receiving
            .entry((key, app_id))
            .or_insert_with(|| (String::new(), 0));
        if index == 0 {
            *entry = (String::new(), 0);
        } else if index != entry.1 {
            return Err(ErrorCode::InvalidOrder);
        }
        if entry.0.len() + text.as_str().len() > limit {
            return Err(ErrorCode::QuotaExceeded);
        }
        entry.0.push_str(text.as_str());
        entry.1 = index.saturating_add(1);
        if last {
            let (complete, _) = receiving.remove(&(key, app_id)).expect("present");
            let names = self.shell_windows.names.entry(key).or_default();
            if app_id {
                names.app_id = complete;
            } else {
                names.title = complete;
            }
        }
        Ok(())
    }

    /// A request of the session's shell, which needs the `shell` grant. A window that has gone
    /// since the shell was told of it is ignored, as is a workspace that is no longer there.
    pub(super) fn shell_request(
        &mut self,
        client: ClientId,
        request: Request,
    ) -> Result<(), ErrorCode> {
        if !self.clients.get(&client).is_some_and(|grants| grants.shell) {
            return Err(ErrorCode::Unauthorized);
        }
        let window = |scene: &Scene, number: u32| scene.window_by_number(number);
        match request {
            Request::FocusWindow { window: number } => {
                if let Some(key) = window(self, number) {
                    self.show_window(key);
                    // With a menu of the shell's open (the top bar's row chosen from the keyboard,
                    // ADR 0024), the window takes the keyboard as the menu closes, not the bar.
                    if self.grab.first().is_some_and(|menu| menu.client == client)
                        && self.focusable(key)
                    {
                        self.focus_before_grab = Some(self.focus_target(key));
                    }
                }
            }
            Request::CloseWindow { window: number } => {
                if let Some(key) = window(self, number) {
                    self.emit(key.client, Event::CloseRequested { surface: key.id });
                }
            }
            Request::MinimizeWindow { window: number } => {
                if let Some(key) = window(self, number) {
                    self.minimize(key);
                }
            }
            Request::MoveWindowToWorkspace {
                window: number,
                workspace,
            } => {
                if let Some(key) = window(self, number) {
                    self.move_to_workspace(key, workspace);
                }
            }
            Request::ActivateWorkspace { workspace } => {
                self.activate_workspace(workspace);
            }
            Request::AddWorkspace {} => {
                self.add_workspace();
            }
            Request::RemoveWorkspace { workspace } => {
                self.remove_workspace(workspace);
            }
            Request::HideApplication { window: number } => {
                if let Some(key) = window(self, number) {
                    self.hide_application(key);
                }
            }
            Request::QuitApplication { window: number } => {
                if let Some(key) = window(self, number) {
                    self.quit_application(key);
                }
            }
            Request::ForceQuitApplication { window: number } => {
                if let Some(key) = window(self, number) {
                    self.force_quit_application(key)?;
                }
            }
            Request::FocusShellSurface { surface, serial } => {
                self.focus_shell_surface(
                    SurfaceKey {
                        client,
                        id: surface,
                    },
                    serial,
                )?;
            }
            _ => unreachable!("only the shell's requests are passed here"),
        }
        Ok(())
    }

    /// Shows a window and activates it: its workspace shown, the window restored (and with it
    /// its application's windows hidden with it).
    pub(super) fn show_window(&mut self, key: SurfaceKey) {
        let node = &self.nodes[&key];
        let (workspace, hidden) = (node.workspace, node.hidden);
        self.activate_workspace(workspace);
        if hidden {
            self.restore(key);
        } else {
            self.activate(key);
        }
    }

    /// What the shell sees now: each mapped window by its number.
    fn shell_view(&self) -> BTreeMap<u32, Window> {
        let focused = self.focus.map(|f| self.root_of(f));
        self.shell_windows
            .numbers
            .iter()
            .filter_map(|(key, number)| {
                let node = self.live(*key).filter(|node| node.mapped)?;
                let mut names = self
                    .shell_windows
                    .names
                    .get(key)
                    .cloned()
                    .unwrap_or_default();
                // A launched process's windows are its registry application's.
                let launched = self.shell_windows.applications.get(&key.client);
                if let Some(id) = launched {
                    names.app_id = id.clone();
                }
                let (used, hidden) = self.window_use(*key);
                Some((
                    *number,
                    Window {
                        workspace: node.workspace,
                        minimized: node.hidden,
                        focused: focused == Some(*key),
                        launched: launched.is_some(),
                        names,
                        size: node.size,
                        used,
                        hidden,
                        responding: self.responding(key.client),
                    },
                ))
            })
            .collect()
    }

    /// Tells each shell client what changed since it was last told (before events are sent).
    pub(super) fn tell_shell(&mut self) {
        let shells: Vec<ClientId> = self
            .clients
            .iter()
            .filter(|(_, grants)| grants.shell)
            .map(|(client, _)| *client)
            .collect();
        if shells.is_empty() {
            return;
        }
        let now = self.shell_view();
        let workspaces = (self.workspaces.count, self.workspaces.active);
        // The first output of the arrangement is the primary.
        let outputs: BTreeMap<OutputId, (Output, bool)> = self
            .outputs
            .iter()
            .enumerate()
            .map(|(index, output)| (output.id, (*output, index == 0)))
            .collect();
        for client in shells {
            let told = self.shell_windows.told.remove(&client).unwrap_or_default();
            // Outputs first: a shell places what it shows on them.
            for id in told.outputs.keys().filter(|id| !outputs.contains_key(id)) {
                self.emit(client, Event::OutputRemoved { output: *id });
            }
            for (id, &(output, primary)) in &outputs {
                if told.outputs.get(id) != Some(&(output, primary)) {
                    self.emit(
                        client,
                        Event::Output {
                            output: *id,
                            rect: output.rect,
                            scale_120: output.scale_120,
                            refresh_mhz: output.refresh_mhz,
                            primary,
                        },
                    );
                }
            }
            for number in told.windows.keys().filter(|n| !now.contains_key(n)) {
                self.emit(client, Event::WindowClosed { window: *number });
            }
            for (&number, window) in &now {
                let before = told.windows.get(&number);
                if before.is_none_or(|b| {
                    (
                        b.workspace,
                        b.minimized,
                        b.focused,
                        b.launched,
                        b.responding,
                    ) != (
                        window.workspace,
                        window.minimized,
                        window.focused,
                        window.launched,
                        window.responding,
                    )
                }) {
                    self.emit(
                        client,
                        Event::WindowState {
                            window: number,
                            workspace: window.workspace,
                            minimized: window.minimized,
                            focused: window.focused,
                            launched: window.launched,
                            responding: window.responding,
                        },
                    );
                }
                if before.is_none_or(|b| (b.used, b.hidden) != (window.used, window.hidden)) {
                    self.emit(
                        client,
                        Event::WindowUse {
                            window: number,
                            used: window.used,
                            hidden: window.hidden,
                        },
                    );
                }
                if before.is_none_or(|b| b.names.title != window.names.title) {
                    self.emit_name(client, number, false, &window.names.title);
                }
                if before.is_none_or(|b| b.names.app_id != window.names.app_id) {
                    self.emit_name(client, number, true, &window.names.app_id);
                }
                if before.is_none_or(|b| b.size != window.size) {
                    self.emit(
                        client,
                        Event::WindowSize {
                            window: number,
                            size: window.size,
                        },
                    );
                }
            }
            if told.workspaces != Some(workspaces) {
                let (count, active) = workspaces;
                self.emit(client, Event::Workspaces { count, active });
            }
            self.shell_windows.told.insert(
                client,
                Told {
                    windows: now.clone(),
                    workspaces: Some(workspaces),
                    outputs: outputs.clone(),
                },
            );
        }
    }

    /// A title or application id in chunks, one (empty) for none.
    fn emit_name(&mut self, client: ClientId, window: u32, app_id: bool, text: &str) {
        let mut chunks: Vec<PreeditText> = TextChunk::split(text)
            .iter()
            .map(|chunk| PreeditText::new(chunk.as_str()).expect("a chunk fits"))
            .collect();
        if chunks.is_empty() {
            chunks.push(PreeditText::new("").expect("empty"));
        }
        let count = chunks.len();
        for (index, text) in chunks.into_iter().enumerate() {
            let (index, last) = (index as u8, index + 1 == count);
            let event = if app_id {
                Event::WindowAppId {
                    window,
                    index,
                    last,
                    text,
                }
            } else {
                Event::WindowTitle {
                    window,
                    index,
                    last,
                    text,
                }
            };
            self.emit(client, event);
        }
    }
}
