//! The install service's socket, `/run/oxyos/installd/socket` (docs/adr/0030-installing-
//! applications.mdx, decision 5): `SOCK_SEQPACKET`, one JSON message per packet of at most 64 KiB,
//! the peer's credentials from the kernel (`SO_PEERCRED`), as `oxyos-accountd`'s (ADR 0020).
//!
//! - **Who acts for whom.** A user's process acts for its own account alone; a request naming
//!   another account (`uid`) is refused. Root acts for the account it names, and must name one
//!   to install or remove; its `list` without one lists nothing. A process of no account (no
//!   `passwd` entry) may do nothing but check for updates.
//! - **Every answer follows the message it answers**, and the service never closes a connection
//!   with a message unread. A connection lasts until its peer closes it, so a session keeps one
//!   for its versions' holds, which end with it.
//! - **The conversation:**
//!
//!   ```text
//!   → {"prepare": {"id", "uid"?}}           ← {"plan": {"planId", …}}
//!   → {"commit": {"planId"}}                ← {"progress": …}…, then {"committed": {…}}
//!   → {"uninstall": {"id", "uid"?}}         ← {"uninstalled": {"id", "dataDirectories"}}
//!   → {"list": {"uid"?}}                    ← {"applications": […]}
//!   → {"hold": {"id", "versionCode"}}       ← "held"
//!   → {"release": {"id", "versionCode"}}    ← "released"
//!   → {"shown": {"id", "versionCode"}}      ← "noted"
//!   → {"exited": {"id", "versionCode", "shown"}}  ← "noted" | "rolledBack"
//!   → "checkUpdates"                        ← {"updates": […]}
//!   ← {"refused": {"reason", "detail"?}}    for any of them
//!   ```
//!
//!   A plan is the connection's: `commit` names the last one `prepare` answered on it. The person
//!   confirmed it in the system's sheet (ADR 0030, decision 9): the session commits it.

use crate::accounts::Accounts;
use crate::compile::Compile;
use crate::fetch::Fetch;
use crate::install::{
    Committed, InstallError, Installer, Listed, Plan, Prepared, Progress, Update,
};
use crate::socket::{Connection, Peer};
use serde::{Deserialize, Serialize};
use std::io;

/// Where the service listens.
pub const SOCKET: &str = "/run/oxyos/installd/socket";

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum Request {
    Prepare {
        id: String,
        #[serde(default)]
        uid: Option<u32>,
    },
    #[serde(rename_all = "camelCase")]
    Commit {
        plan_id: String,
    },
    Uninstall {
        id: String,
        #[serde(default)]
        uid: Option<u32>,
    },
    List {
        #[serde(default)]
        uid: Option<u32>,
    },
    #[serde(rename_all = "camelCase")]
    Hold {
        id: String,
        version_code: u64,
    },
    #[serde(rename_all = "camelCase")]
    Release {
        id: String,
        version_code: u64,
    },
    #[serde(rename_all = "camelCase")]
    Shown {
        id: String,
        version_code: u64,
    },
    #[serde(rename_all = "camelCase")]
    Exited {
        id: String,
        version_code: u64,
        shown: bool,
    },
    CheckUpdates,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    NotPermitted,
    InvalidRequest,
    NoPlan,
    NoRelease,
    NotInstalled,
    Downgrade,
    RolledBack,
    Stale,
    Store,
    Package,
    Release,
    Fetch,
    Compile,
    Failed,
}

/// A plan as the session is told it: the plan and its id on this connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanAnswer {
    pub plan_id: String,
    #[serde(flatten)]
    pub plan: Plan,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum Answer {
    Plan(PlanAnswer),
    Progress(Progress),
    Committed(Committed),
    #[serde(rename_all = "camelCase")]
    Uninstalled {
        id: String,
        data_directories: Vec<String>,
    },
    Applications(Vec<Listed>),
    Held,
    Released,
    Noted,
    RolledBack,
    Updates(Vec<Update>),
    Refused {
        reason: Reason,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
}

impl Answer {
    pub fn refused(reason: Reason) -> Self {
        Answer::Refused {
            reason,
            detail: None,
        }
    }
}

impl From<&InstallError> for Answer {
    fn from(error: &InstallError) -> Self {
        let reason = match error {
            InstallError::NotPermitted => Reason::NotPermitted,
            InstallError::NoRelease => Reason::NoRelease,
            InstallError::NotInstalled => Reason::NotInstalled,
            InstallError::Downgrade => Reason::Downgrade,
            InstallError::RolledBack => Reason::RolledBack,
            InstallError::Stale => Reason::Stale,
            InstallError::Store(_) => Reason::Store,
            InstallError::Package(_) => Reason::Package,
            InstallError::Release(_) => Reason::Release,
            InstallError::Fetch(_) => Reason::Fetch,
            InstallError::Compile(_) => Reason::Compile,
            InstallError::Io(_) => Reason::Failed,
        };
        Answer::Refused {
            reason,
            detail: Some(error.to_string()),
        }
    }
}

/// What a turn of a conversation did, for the service's log.
#[derive(Debug)]
pub struct Outcome {
    pub request: &'static str,
    pub id: Option<String>,
    pub result: Result<String, String>,
}

/// One connection's conversation: its last plan and the versions it holds.
pub struct Conversation<'a, F, C, A> {
    installer: &'a Installer<F, C, A>,
    peer: Peer,
    plan: Option<(String, Prepared)>,
    holds: Vec<(String, u64)>,
}

impl<'a, F: Fetch, C: Compile, A: Accounts> Conversation<'a, F, C, A> {
    pub fn new(installer: &'a Installer<F, C, A>, peer: Peer) -> Self {
        Self {
            installer,
            peer,
            plan: None,
            holds: Vec::new(),
        }
    }

    /// For whom a request acts: the peer's own account, or the one root names.
    fn account(&self, uid: Option<u32>) -> Result<u32, Answer> {
        match (self.peer.uid, uid) {
            (0, Some(uid)) => Ok(uid),
            (0, None) => Err(Answer::refused(Reason::InvalidRequest)),
            (peer, None) => Ok(peer),
            (peer, Some(uid)) if uid == peer => Ok(peer),
            _ => Err(Answer::refused(Reason::NotPermitted)),
        }
    }

    /// Whether the peer may say what a version of `id` does (a session of an account that has it).
    fn runs(&self, id: &str) -> bool {
        self.installer.has(self.peer.uid, id)
    }

    /// Answers a message (`None`: one that could not be read). `send` carries the progress
    /// answers before the last one.
    pub fn answer(
        &mut self,
        request: Option<Request>,
        send: &mut dyn FnMut(&Answer) -> io::Result<()>,
    ) -> (Answer, Outcome) {
        let outcome = |request, id: Option<&str>, result: Result<String, String>| Outcome {
            request,
            id: id.map(str::to_string),
            result,
        };
        let refusal = |answer: Answer, name, id: Option<&str>| {
            let detail = format!("{answer:?}");
            (answer, outcome(name, id, Err(detail)))
        };
        let failed = |error: &InstallError, name, id: Option<&str>| {
            (
                Answer::from(error),
                outcome(name, id, Err(error.to_string())),
            )
        };
        let Some(request) = request else {
            return refusal(Answer::refused(Reason::InvalidRequest), "unreadable", None);
        };
        match request {
            Request::Prepare { id, uid } => {
                let uid = match self.account(uid) {
                    Ok(uid) => uid,
                    Err(answer) => return refusal(answer, "prepare", Some(&id)),
                };
                match self.installer.prepare(uid, &id) {
                    Ok(prepared) => {
                        let plan_id = random_id();
                        let answer = Answer::Plan(PlanAnswer {
                            plan_id: plan_id.clone(),
                            plan: prepared.plan.clone(),
                        });
                        let what = format!("version {}", prepared.plan.version_code);
                        self.plan = Some((plan_id, prepared));
                        (answer, outcome("prepare", Some(&id), Ok(what)))
                    }
                    Err(e) => failed(&e, "prepare", Some(&id)),
                }
            }
            Request::Commit { plan_id } => {
                let Some((_, prepared)) = self.plan.take().filter(|(p, _)| *p == plan_id) else {
                    return refusal(Answer::refused(Reason::NoPlan), "commit", None);
                };
                let id = prepared.plan.id.clone();
                let mut progress = |p: Progress| {
                    let _ = send(&Answer::Progress(p));
                };
                match self.installer.commit(&prepared, &mut progress) {
                    Ok(committed) => {
                        let what = format!(
                            "version {}{}",
                            committed.version_code,
                            if committed.switched { "" } else { ", staged" }
                        );
                        (
                            Answer::Committed(committed),
                            outcome("commit", Some(&id), Ok(what)),
                        )
                    }
                    Err(e) => failed(&e, "commit", Some(&id)),
                }
            }
            Request::Uninstall { id, uid } => {
                let uid = match self.account(uid) {
                    Ok(uid) => uid,
                    Err(answer) => return refusal(answer, "uninstall", Some(&id)),
                };
                match self.installer.uninstall(uid, &id) {
                    Ok(data_directories) => (
                        Answer::Uninstalled {
                            id: id.clone(),
                            data_directories,
                        },
                        outcome("uninstall", Some(&id), Ok(format!("for uid {uid}"))),
                    ),
                    Err(e) => failed(&e, "uninstall", Some(&id)),
                }
            }
            Request::List { uid } => {
                let uid = match (self.peer.uid, uid) {
                    (0, None) => {
                        return (
                            Answer::Applications(Vec::new()),
                            outcome("list", None, Ok("nothing".into())),
                        )
                    }
                    _ => match self.account(uid) {
                        Ok(uid) => uid,
                        Err(answer) => return refusal(answer, "list", None),
                    },
                };
                match self.installer.list(uid) {
                    Ok(listed) => {
                        let n = listed.len();
                        (
                            Answer::Applications(listed),
                            outcome("list", None, Ok(format!("{n}"))),
                        )
                    }
                    Err(e) => failed(&e, "list", None),
                }
            }
            Request::Hold { id, version_code } => {
                if !self.runs(&id) {
                    return refusal(Answer::refused(Reason::NotPermitted), "hold", Some(&id));
                }
                match self.installer.hold(&id, version_code) {
                    Ok(()) => {
                        self.holds.push((id.clone(), version_code));
                        (
                            Answer::Held,
                            outcome("hold", Some(&id), Ok(format!("{version_code}"))),
                        )
                    }
                    Err(e) => failed(&e, "hold", Some(&id)),
                }
            }
            Request::Release { id, version_code } => {
                let Some(at) = self
                    .holds
                    .iter()
                    .position(|(i, c)| *i == id && *c == version_code)
                else {
                    return refusal(Answer::refused(Reason::NotInstalled), "release", Some(&id));
                };
                self.holds.remove(at);
                match self.installer.release(&id, version_code) {
                    Ok(()) => (
                        Answer::Released,
                        outcome("release", Some(&id), Ok(format!("{version_code}"))),
                    ),
                    Err(e) => failed(&e, "release", Some(&id)),
                }
            }
            Request::Shown { id, version_code } => {
                if !self.runs(&id) {
                    return refusal(Answer::refused(Reason::NotPermitted), "shown", Some(&id));
                }
                match self.installer.shown(&id, version_code) {
                    Ok(()) => (
                        Answer::Noted,
                        outcome("shown", Some(&id), Ok(format!("{version_code}"))),
                    ),
                    Err(e) => failed(&e, "shown", Some(&id)),
                }
            }
            Request::Exited {
                id,
                version_code,
                shown,
            } => {
                if !self.runs(&id) {
                    return refusal(Answer::refused(Reason::NotPermitted), "exited", Some(&id));
                }
                match self.installer.exited(&id, version_code, shown) {
                    Ok(true) => (
                        Answer::RolledBack,
                        outcome(
                            "exited",
                            Some(&id),
                            Ok(format!("{version_code} rolled back")),
                        ),
                    ),
                    Ok(false) => (
                        Answer::Noted,
                        outcome("exited", Some(&id), Ok(format!("{version_code}"))),
                    ),
                    Err(e) => failed(&e, "exited", Some(&id)),
                }
            }
            Request::CheckUpdates => match self.installer.check_updates() {
                Ok(updates) => {
                    let n = updates.len();
                    (
                        Answer::Updates(updates),
                        outcome("checkUpdates", None, Ok(format!("{n} applications"))),
                    )
                }
                Err(e) => failed(&e, "checkUpdates", None),
            },
        }
    }

    /// The conversation ended: what it held is released. [`serve`] calls it however the
    /// connection ended.
    pub fn end(&mut self) {
        for (id, code) in std::mem::take(&mut self.holds) {
            let _ = self.installer.release(&id, code);
        }
    }
}

/// Runs one connection's conversation to its end; each turn's outcome goes to `log`.
pub fn serve<F: Fetch, C: Compile, A: Accounts>(
    connection: &Connection,
    installer: &Installer<F, C, A>,
    mut log: impl FnMut(&Peer, &Outcome),
) -> io::Result<()> {
    let peer = connection.peer()?;
    let mut conversation = Conversation::new(installer, peer);
    let result = (|| loop {
        let Some(request) = connection.receive::<Request>()? else {
            return Ok(());
        };
        let (answer, outcome) =
            conversation.answer(request, &mut |progress| connection.send(progress));
        log(&peer, &outcome);
        connection.send(&answer)?;
    })();
    conversation.end();
    result
}

fn random_id() -> String {
    use ring::rand::SecureRandom;
    let mut bytes = [0u8; 16];
    let _ = ring::rand::SystemRandom::new().fill(&mut bytes);
    crate::tuf::hex_of(&bytes)
}
