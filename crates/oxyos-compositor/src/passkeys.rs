//! Passkeys' sheet for the program running the compositor
//! (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 7): the person's verification of
//! every passkey an application creates or uses, in a system sheet over the application's window.
//!
//! The accounts' service asks (a [`PasskeyPrompts`] source: the session's registrar connection
//! for the sheets, or the development compositor's fixture): which application (its launch
//! record's desktop entry id and its process), which relying party, which account, and whether a
//! passkey is to be made (`create`), used (`get`, a chooser when several match), or only said to
//! be there already (`excluded`) or not there at all (`none`). This shows the sheet the shell
//! draws, over that application's window: the session's sheet (`kind: "passkey"`), sent to the
//! shell alone, which no application draws, reads or answers.
//!
//! - **The answer.** A `create` or a `get` is answered only with the account's password, typed in
//!   the sheet's secure field: the shell sends `{"passkey": {"sheet", "choice"?, "password"}}`
//!   citing the input that submitted it, which is taken only from the shell, only for a passkey
//!   sheet it shows, and never while locked, and handed to the service, which checks it. A wrong
//!   one is told back (`passkeyRetry`): the sheet is shown again saying so (a new sheet, its
//!   field empty), or saying how long to wait. An `excluded` or `none` sheet only tells, and is
//!   answered by its one button. Cancel, Escape, or the application going is a cancel.
//! - **Only for its own application's window.** The asking process must be a client the session
//!   started as that desktop entry id (its launch record); otherwise nothing is shown and the
//!   ceremony is cancelled. The password is never told to anyone but the service.

use crate::autofill::Secret;
use crate::Compositor;
use oxyos_compositor_core::scene::SystemRequest;
use oxyos_compositor_core::{ClientId, Scene};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

/// What the sheet asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Make a passkey: the password.
    Create,
    /// Use one: the password, and which one when several.
    Get,
    /// The person already has a passkey there: told.
    Excluded,
    /// No passkey there: told.
    None,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Create => "create",
            Kind::Get => "get",
            Kind::Excluded => "excluded",
            Kind::None => "none",
        }
    }

    /// Whether the person answers it with the account's password.
    fn verifies(self) -> bool {
        matches!(self, Kind::Create | Kind::Get)
    }
}

/// A passkey the person may choose: its account's name and display name. `Debug` shows neither.
#[derive(Clone, PartialEq, Eq)]
pub struct Choice {
    pub user_name: String,
    pub display_name: String,
}

impl std::fmt::Debug for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Choice(..)")
    }
}

/// What the service tells the session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Prompt {
    /// A sheet to show.
    Confirm {
        id: String,
        desktop_entry_id: String,
        pid: i32,
        rp_id: String,
        /// The account the relying party names: `Debug` does not show it.
        user_name: Option<Secret>,
        kind: Kind,
        choices: Vec<Choice>,
    },
    /// The password was not right (`throttled`: too many were not; wait `retry_after` seconds).
    Retry {
        id: String,
        throttled: bool,
        retry_after: Option<u64>,
    },
    /// The ceremony ended: its sheet goes.
    Done { id: String },
}

/// The person's answer, for the service.
#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
    Confirm {
        id: String,
        choice: Option<usize>,
        password: Option<Secret>,
    },
    Cancel {
        id: String,
    },
}

/// Where the prompts come from and the answers go: the accounts' service on the session's
/// registrar connection for its sheets (on a thread of its own, waking the turn), or a fixture's.
/// Neither ever makes the compositor's turn wait.
pub trait PasskeyPrompts {
    /// What the service told since the last call, in order.
    fn take(&mut self) -> Vec<Prompt>;
    /// Hands the person's answer to the service.
    fn reply(&mut self, reply: Reply);
}

/// A ceremony shown: what its sheet says, and the sheet showing it.
struct Shown {
    text: Value,
    client: ClientId,
    kind: Kind,
    choices: usize,
    sheet: u32,
}

/// The shell's answer, read strictly.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
enum ShellAsk {
    Passkey {
        sheet: u32,
        #[serde(default)]
        choice: Option<usize>,
        password: String,
    },
}

fn ours(text: &str) -> bool {
    serde_json::from_str::<HashMap<String, serde::de::IgnoredAny>>(text)
        .is_ok_and(|asked| asked.len() == 1 && asked.contains_key("passkey"))
}

/// What a sheet answered by its buttons says: an acknowledgement of one that only tells, else a
/// cancel (a sheet that asks for the password is answered with it, never by a bare button).
fn settled(id: String, kind: Kind, accepted: bool) -> Reply {
    if accepted && !kind.verifies() {
        Reply::Confirm {
            id,
            choice: None,
            password: None,
        }
    } else {
        Reply::Cancel { id }
    }
}

fn refused_text(reason: &str) -> String {
    json!({"refused": {"reason": reason}}).to_string()
}

pub struct Passkeys {
    prompts: Box<dyn PasskeyPrompts>,
    /// The ceremonies shown, by the service's id.
    shown: HashMap<String, Shown>,
}

impl Passkeys {
    pub fn new(prompts: Box<dyn PasskeyPrompts>) -> Self {
        Self {
            prompts,
            shown: HashMap::new(),
        }
    }

    /// A turn: the service's prompts, the shell's answers; sent at once.
    pub fn serve(&mut self, compositor: &mut Compositor) {
        let peers: Vec<(ClientId, i32)> = compositor
            .peers
            .iter()
            .map(|(client, peer)| (*client, peer.pid))
            .collect();
        let client_of = |pid: i32| {
            peers
                .iter()
                .find(|(_, held)| *held == pid)
                .map(|(client, _)| *client)
        };
        if self.serve_scene(&mut compositor.scene, &client_of) {
            compositor.flush();
        }
    }

    /// The same on the scene alone, `client_of` naming the client of a process; whether anything
    /// was shown or withdrawn.
    pub fn serve_scene(
        &mut self,
        scene: &mut Scene,
        client_of: &dyn Fn(i32) -> Option<ClientId>,
    ) -> bool {
        let mut changed = false;
        for request in scene.take_system_requests_where(|request| ours(&request.text)) {
            self.handle(scene, request);
            changed = true;
        }
        // A sheet answered: a cancel, or an acknowledgement of one that only tells. A sheet that
        // asks for the password is answered with it (above), never accepted bare.
        let answered: Vec<(String, Kind, bool)> = self
            .shown
            .iter()
            .filter_map(|(id, shown)| {
                scene
                    .take_sheet_answer(shown.sheet)
                    .map(|accepted| (id.clone(), shown.kind, accepted))
            })
            .collect();
        for (id, kind, accepted) in answered {
            self.shown.remove(&id);
            self.prompts.reply(settled(id, kind, accepted));
        }
        for prompt in self.prompts.take() {
            changed = true;
            match prompt {
                Prompt::Confirm {
                    id,
                    desktop_entry_id,
                    pid,
                    rp_id,
                    user_name,
                    kind,
                    choices,
                } => {
                    // Over its own application's window: a client the session started as that
                    // application, else nothing is shown.
                    let client = client_of(pid)
                        .filter(|client| scene.application(*client) == Some(&desktop_entry_id));
                    let Some(client) = client else {
                        eprintln!(
                            "[passkeys] {desktop_entry_id} asked for a passkey without a window of its own"
                        );
                        self.prompts.reply(Reply::Cancel { id });
                        continue;
                    };
                    let mut text = json!({
                        "kind": "passkey",
                        "mode": kind.name(),
                        "id": desktop_entry_id,
                        "rpId": rp_id,
                        "choices": choices.iter().map(|choice| json!({
                            "userName": choice.user_name,
                            "displayName": choice.display_name,
                        })).collect::<Vec<_>>(),
                        "incorrect": false,
                    });
                    if let Some(user_name) = user_name {
                        text["userName"] = json!(user_name.expose());
                    }
                    self.show(
                        scene,
                        id,
                        Shown {
                            text,
                            client,
                            kind,
                            choices: choices.len(),
                            sheet: 0,
                        },
                    );
                }
                Prompt::Retry {
                    id,
                    throttled,
                    retry_after,
                } => {
                    let Some(mut shown) = self.shown.remove(&id) else {
                        continue;
                    };
                    scene.dismiss_sheet(shown.sheet);
                    shown.text["incorrect"] = json!(!throttled);
                    match retry_after.filter(|_| throttled) {
                        Some(seconds) => shown.text["retryAfter"] = json!(seconds),
                        None => {
                            if let Some(text) = shown.text.as_object_mut() {
                                text.remove("retryAfter");
                            }
                        }
                    }
                    self.show(scene, id, shown);
                }
                Prompt::Done { id } => {
                    if let Some(shown) = self.shown.remove(&id) {
                        scene.dismiss_sheet(shown.sheet);
                    }
                }
            }
        }
        changed
    }

    /// Shows a ceremony's sheet (again), or cancels it when no sheet can be shown.
    fn show(&mut self, scene: &mut Scene, id: String, mut shown: Shown) {
        match scene.show_sheet(Some(shown.client), &shown.text.to_string()) {
            Some(sheet) => {
                shown.sheet = sheet;
                self.shown.insert(id, shown);
            }
            None => self.prompts.reply(Reply::Cancel { id }),
        }
    }

    /// The shell's answer to a passkey sheet (a request of this part's vocabulary): the password
    /// typed, and the passkey chosen.
    pub fn handle(&mut self, scene: &mut Scene, mut request: SystemRequest) {
        let parsed = serde_json::from_str::<ShellAsk>(&request.text);
        // The password goes with the request's text: cleared, it is held only as a Secret.
        zeroize::Zeroize::zeroize(&mut request.text);
        let (client, serial) = (request.client, request.serial);
        let Ok(ShellAsk::Passkey {
            sheet,
            choice,
            password,
        }) = parsed
        else {
            scene.system_text(client, serial, &refused_text("invalidRequest"));
            return;
        };
        let password = Secret::new(password);
        if !scene.grants_of(client).is_some_and(|grants| grants.shell) {
            scene.system_text(client, serial, &refused_text("notPermitted"));
            return;
        }
        if !request.with_input {
            scene.system_text(client, serial, &refused_text("needsInput"));
            return;
        }
        if scene.is_locked() {
            scene.system_text(client, serial, &refused_text("locked"));
            return;
        }
        let found = self
            .shown
            .iter()
            .find(|(_, shown)| shown.sheet == sheet && shown.kind.verifies())
            .map(|(id, shown)| (id.clone(), shown.kind, shown.choices));
        let Some((id, kind, choices)) = found else {
            scene.system_text(client, serial, &refused_text("noSuchSheet"));
            return;
        };
        // A get names one of its passkeys (the first when it does not say); a create none.
        let choice = match kind {
            Kind::Get => match choice.unwrap_or(0) {
                index if index < choices => Some(index),
                _ => {
                    scene.system_text(client, serial, &refused_text("invalidRequest"));
                    return;
                }
            },
            _ => None,
        };
        // The sheet stays until the service says the ceremony is done or asks again; asked again,
        // it keeps the account chosen.
        if let (Some(shown), Some(choice)) = (self.shown.get_mut(&id), choice) {
            shown.text["chosen"] = json!(choice);
        }
        self.prompts.reply(Reply::Confirm {
            id,
            choice,
            password: Some(password),
        });
    }
}

#[cfg(test)]
mod tests;
