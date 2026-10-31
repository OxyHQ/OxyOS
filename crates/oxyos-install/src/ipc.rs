//! The install service's socket, `/run/oxyos/installd/socket` (docs/adr/0030-installing-
//! applications.mdx, decisions 5, 10 and 16): `SOCK_SEQPACKET`, one JSON message per packet of at
//! most 64 KiB, the peer's credentials from the kernel (`SO_PEERCRED`), as `oxyos-accountd`'s.
//!
//! - **Every request is an account's.** A user's process acts for its own account alone, and a
//!   request naming another account (`uid`) is refused. Root acts for the account it names, and
//!   must name one; without one it may only check, and run the automatic updates.
//! - **Every answer follows the message it answers**, and the service never closes a connection
//!   with a message unread. A connection lasts until its peer closes it: a session keeps one for
//!   its versions' holds and its watch, which end with it.
//! - **The conversation:**
//!
//!   ```text
//!   → {"resolve": {"listings", "uid"?}}              ← {"resolved": {listing: id | null}}
//!   → {"prepare": {"id", "uid"?}}                    ← {"plan": {"planId", …}}
//!   → {"commit": {"planId"}}                         ← {"progress": …}…, then {"committed": {…}}
//!   → {"uninstall": {"id", "uid"?}}                  ← {"uninstalled": {"id", "dataDirectories"}}
//!   → {"list": {"uid"?}}                             ← {"applications": […]}
//!   → {"hold"|"release"|"shown": {"id", "versionCode", "uid"?}}  ← "held" | "released" | "noted"
//!   → {"exited": {"id", "versionCode", "shown", "uid"?}}         ← "noted" | "rolledBack"
//!   → {"checkUpdates": {"uid"?}}                     ← {"updateState": {…}} (root: "checked")
//!   → {"updateState": {"uid"?}}                      ← {"updateState": {…}}
//!   → {"watchUpdates": {"uid"?}}                     ← "watching", then {"updateState": {…}} now
//!                                                      and at every change, unasked
//!   → {"update": {"ids", "uid"?}}                    ← {"updated": [{"id", "versionCode",
//!   → {"updateAll": {"uid"?}}                            "outcome", "reason"}]}
//!   → {"consent": {"id", "versionCode", "uid"?}}     ← {"updated": [{…}]}
//!   → {"setAutomatic": {"apps", "uid"?}}             ← "automaticSet"
//!   → "automaticUpdates"                             ← {"automatic": [{"uid", "updated"}]} (root)
//!   ← {"refused": {"reason", "detail"?}}             for any of them
//!   ```
//!
//!   A plan is the connection's: `commit` names the last one `prepare` answered on it. The person
//!   confirmed it in the system's sheet (decision 9), which is the account's consent; so is
//!   `consent`. A watching connection is sent its account's state unasked, between answers.

use crate::accounts::Accounts;
use crate::compile::Compile;
use crate::fetch::Fetch;
use crate::install::{
    Committed, InstallError, Installer, Listed, Plan, Prepared, Progress, UpdateState, Updated,
};
use crate::socket::{Connection, Peer};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::time::Duration;

/// Where the service listens.
pub const SOCKET: &str = "/run/oxyos/installd/socket";

/// How long a watcher's peer may take to read a change before the watch ends.
pub const WATCH_SEND: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum Request {
    /// The application each store listing (its slug) is on this machine, from the store's
    /// signed metadata: what a store names an application by, never a package's bytes.
    Resolve {
        listings: Vec<String>,
        #[serde(default)]
        uid: Option<u32>,
    },
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
        #[serde(default)]
        uid: Option<u32>,
    },
    #[serde(rename_all = "camelCase")]
    Release {
        id: String,
        version_code: u64,
        #[serde(default)]
        uid: Option<u32>,
    },
    #[serde(rename_all = "camelCase")]
    Shown {
        id: String,
        version_code: u64,
        #[serde(default)]
        uid: Option<u32>,
    },
    #[serde(rename_all = "camelCase")]
    Exited {
        id: String,
        version_code: u64,
        shown: bool,
        #[serde(default)]
        uid: Option<u32>,
    },
    CheckUpdates {
        #[serde(default)]
        uid: Option<u32>,
    },
    UpdateState {
        #[serde(default)]
        uid: Option<u32>,
    },
    WatchUpdates {
        #[serde(default)]
        uid: Option<u32>,
    },
    Update {
        ids: Vec<String>,
        #[serde(default)]
        uid: Option<u32>,
    },
    UpdateAll {
        #[serde(default)]
        uid: Option<u32>,
    },
    #[serde(rename_all = "camelCase")]
    Consent {
        id: String,
        version_code: u64,
        #[serde(default)]
        uid: Option<u32>,
    },
    SetAutomatic {
        apps: bool,
        #[serde(default)]
        uid: Option<u32>,
    },
    AutomaticUpdates,
    /// The account's decision on a capability its installed version declares (decision 8): from
    /// the system's sheet at first use, or Settings' Privacy page. Kept in the account's record,
    /// root's, so no process of the account grants itself one by writing a file.
    #[serde(rename_all = "camelCase")]
    Decide {
        id: String,
        capability: String,
        granted: bool,
        #[serde(default)]
        uid: Option<u32>,
    },
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

/// What the automatic updates did for an account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AutomaticAnswer {
    pub uid: u32,
    pub updated: Vec<Updated>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum Answer {
    /// Each listing asked, and the package id it is on this machine, or null.
    Resolved(BTreeMap<String, Option<String>>),
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
    Checked,
    UpdateState(UpdateState),
    Watching,
    Updated(Vec<Updated>),
    AutomaticSet,
    Automatic(Vec<AutomaticAnswer>),
    Decided,
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
            InstallError::InvalidRequest(_) => Reason::InvalidRequest,
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

/// One connection's conversation: its last plan, the versions it holds and its watches.
pub struct Conversation<'a, F, C, A> {
    installer: &'a Installer<F, C, A>,
    peer: Peer,
    /// A handle to send its watches' changes on.
    watch_on: Option<Connection>,
    plan: Option<(String, Prepared)>,
    holds: Vec<(u32, String, u64)>,
    watches: Vec<u64>,
}

impl<'a, F: Fetch, C: Compile, A: Accounts> Conversation<'a, F, C, A> {
    /// A conversation with `peer`; `watch_on` is a handle on its connection, for watches.
    pub fn new(
        installer: &'a Installer<F, C, A>,
        peer: Peer,
        watch_on: Option<Connection>,
    ) -> Self {
        Self {
            installer,
            peer,
            watch_on,
            plan: None,
            holds: Vec::new(),
            watches: Vec::new(),
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

    /// Answers a message (`None`: one that could not be read). `send` carries the progress
    /// answers before the last one.
    pub fn answer(
        &mut self,
        request: Option<Request>,
        send: &mut dyn FnMut(&Answer) -> io::Result<()>,
    ) -> (Answer, Outcome) {
        let Some(request) = request else {
            return refusal(Answer::refused(Reason::InvalidRequest), "unreadable", None);
        };
        // The account a request acts for, or its refusal.
        macro_rules! account {
            ($uid:expr, $name:expr, $id:expr) => {
                match self.account($uid) {
                    Ok(uid) => uid,
                    Err(answer) => return refusal(answer, $name, $id),
                }
            };
        }
        match request {
            Request::Resolve { listings, uid } => {
                let uid = account!(uid, "resolve", None);
                match self.installer.resolve(uid, &listings) {
                    Ok(resolved) => {
                        let found = resolved.values().filter(|id| id.is_some()).count();
                        let what = format!("{found} of {}", resolved.len());
                        (
                            Answer::Resolved(resolved),
                            outcome("resolve", None, Ok(what)),
                        )
                    }
                    Err(e) => failed(&e, "resolve", None),
                }
            }
            Request::Prepare { id, uid } => {
                let uid = account!(uid, "prepare", Some(&id));
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
                let uid = account!(uid, "uninstall", Some(&id));
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
                let uid = account!(uid, "list", None);
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
            Request::Hold {
                id,
                version_code,
                uid,
            } => {
                let uid = account!(uid, "hold", Some(&id));
                match self.installer.hold(uid, &id, version_code) {
                    Ok(()) => {
                        self.holds.push((uid, id.clone(), version_code));
                        (
                            Answer::Held,
                            outcome("hold", Some(&id), Ok(format!("{version_code}"))),
                        )
                    }
                    Err(e) => failed(&e, "hold", Some(&id)),
                }
            }
            Request::Release {
                id,
                version_code,
                uid,
            } => {
                let uid = account!(uid, "release", Some(&id));
                let held = (uid, id.clone(), version_code);
                let Some(at) = self.holds.iter().position(|h| *h == held) else {
                    return refusal(Answer::refused(Reason::NotInstalled), "release", Some(&id));
                };
                self.holds.remove(at);
                match self.installer.release(uid, &id, version_code) {
                    Ok(()) => (
                        Answer::Released,
                        outcome("release", Some(&id), Ok(format!("{version_code}"))),
                    ),
                    Err(e) => failed(&e, "release", Some(&id)),
                }
            }
            Request::Shown {
                id,
                version_code,
                uid,
            } => {
                let uid = account!(uid, "shown", Some(&id));
                match self.installer.shown(uid, &id, version_code) {
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
                uid,
            } => {
                let uid = account!(uid, "exited", Some(&id));
                match self.installer.exited(uid, &id, version_code, shown) {
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
            Request::CheckUpdates { uid } => {
                let account = match (self.peer.uid, uid) {
                    (0, None) => None,
                    _ => Some(account!(uid, "checkUpdates", None)),
                };
                if let Err(e) = self.installer.check() {
                    return failed(&e, "checkUpdates", None);
                }
                // How often the store was read, in the log: two asking at once make one.
                let read = Ok(format!(
                    "checked; the store read {} times in all",
                    self.installer.checks_made()
                ));
                match account {
                    None => (Answer::Checked, outcome("checkUpdates", None, read)),
                    Some(uid) => (
                        self.state(uid, "checkUpdates").0,
                        outcome("checkUpdates", None, read),
                    ),
                }
            }
            Request::UpdateState { uid } => {
                let uid = account!(uid, "updateState", None);
                self.state(uid, "updateState")
            }
            Request::WatchUpdates { uid } => {
                let uid = account!(uid, "watchUpdates", None);
                let Some(on) = self.watch_on.as_ref().and_then(|c| c.try_clone().ok()) else {
                    return refusal(Answer::refused(Reason::Failed), "watchUpdates", None);
                };
                // A send's timeout alone: the clone shares the socket, and a receive timeout here
                // would close the watch once it had been quiet for that long.
                let _ = on.set_send_timeout(WATCH_SEND);
                // The answer goes first: the state follows it, now and at each change.
                if let Err(e) = send(&Answer::Watching) {
                    return refusal(
                        Answer::Refused {
                            reason: Reason::Failed,
                            detail: Some(e.to_string()),
                        },
                        "watchUpdates",
                        None,
                    );
                }
                let watcher = Box::new(move |state: &UpdateState| {
                    on.send(&Answer::UpdateState(state.clone())).is_ok()
                });
                match self.installer.watch(uid, watcher) {
                    Ok(watch) => {
                        self.watches.push(watch);
                        // "watching" was sent; this turn answers nothing more.
                        (
                            Answer::Watching,
                            outcome("watchUpdates", None, Ok(format!("uid {uid}"))),
                        )
                    }
                    Err(e) => failed(&e, "watchUpdates", None),
                }
            }
            Request::Update { ids, uid } => {
                let uid = account!(uid, "update", None);
                self.updated(self.installer.update(uid, Some(&ids)), "update")
            }
            Request::UpdateAll { uid } => {
                let uid = account!(uid, "updateAll", None);
                self.updated(self.installer.update(uid, None), "updateAll")
            }
            Request::Consent {
                id,
                version_code,
                uid,
            } => {
                let uid = account!(uid, "consent", Some(&id));
                self.updated(
                    self.installer
                        .consent(uid, &id, version_code)
                        .map(|u| vec![u]),
                    "consent",
                )
            }
            Request::SetAutomatic { apps, uid } => {
                let uid = account!(uid, "setAutomatic", None);
                match self.installer.set_automatic(uid, apps) {
                    Ok(()) => (
                        Answer::AutomaticSet,
                        outcome("setAutomatic", None, Ok(format!("{apps}"))),
                    ),
                    Err(e) => failed(&e, "setAutomatic", None),
                }
            }
            Request::Decide {
                id,
                capability,
                granted,
                uid,
            } => {
                let uid = account!(uid, "decide", Some(&id));
                match self.installer.decide(uid, &id, &capability, granted) {
                    Ok(()) => (
                        Answer::Decided,
                        outcome("decide", Some(&id), Ok(format!("{capability} {granted}"))),
                    ),
                    Err(e) => failed(&e, "decide", Some(&id)),
                }
            }
            Request::AutomaticUpdates => {
                if self.peer.uid != 0 {
                    return refusal(
                        Answer::refused(Reason::NotPermitted),
                        "automaticUpdates",
                        None,
                    );
                }
                match self.installer.automatic() {
                    Ok(done) => {
                        let n = done.len();
                        (
                            Answer::Automatic(
                                done.into_iter()
                                    .map(|(uid, updated)| AutomaticAnswer { uid, updated })
                                    .collect(),
                            ),
                            outcome("automaticUpdates", None, Ok(format!("{n} accounts"))),
                        )
                    }
                    Err(e) => failed(&e, "automaticUpdates", None),
                }
            }
        }
    }

    fn state(&self, uid: u32, name: &'static str) -> (Answer, Outcome) {
        match self.installer.update_state(uid) {
            Ok(state) => {
                let n = state.updates.len();
                (
                    Answer::UpdateState(state),
                    outcome(name, None, Ok(format!("{n} updates"))),
                )
            }
            Err(e) => failed(&e, name, None),
        }
    }

    fn updated(
        &self,
        result: Result<Vec<Updated>, InstallError>,
        name: &'static str,
    ) -> (Answer, Outcome) {
        match result {
            Ok(updated) => {
                let what = updated
                    .iter()
                    .map(|u| format!("{} {}", u.id, u.outcome))
                    .collect::<Vec<_>>()
                    .join(", ");
                (Answer::Updated(updated), outcome(name, None, Ok(what)))
            }
            Err(e) => failed(&e, name, None),
        }
    }

    /// The conversation ended: what it held is released and its watches end. [`serve`] calls it
    /// however the connection ended.
    pub fn end(&mut self) {
        for watch in std::mem::take(&mut self.watches) {
            self.installer.unwatch(watch);
        }
        for (uid, id, code) in std::mem::take(&mut self.holds) {
            let _ = self.installer.release(uid, &id, code);
        }
    }
}

fn outcome(request: &'static str, id: Option<&str>, result: Result<String, String>) -> Outcome {
    Outcome {
        request,
        id: id.map(str::to_string),
        result,
    }
}

fn refusal(answer: Answer, name: &'static str, id: Option<&str>) -> (Answer, Outcome) {
    let detail = format!("{answer:?}");
    (answer, outcome(name, id, Err(detail)))
}

fn failed(error: &InstallError, name: &'static str, id: Option<&str>) -> (Answer, Outcome) {
    (
        Answer::from(error),
        outcome(name, id, Err(error.to_string())),
    )
}

/// Runs one connection's conversation to its end; each turn's outcome goes to `log`.
pub fn serve<F: Fetch, C: Compile, A: Accounts>(
    connection: &Connection,
    installer: &Installer<F, C, A>,
    mut log: impl FnMut(&Peer, &Outcome),
) -> io::Result<()> {
    let peer = connection.peer()?;
    let mut conversation = Conversation::new(installer, peer, connection.try_clone().ok());
    let result = (|| loop {
        let Some(request) = connection.receive::<Request>()? else {
            return Ok(());
        };
        let watching = matches!(request, Some(Request::WatchUpdates { .. }));
        let (answer, outcome) =
            conversation.answer(request, &mut |progress| connection.send(progress));
        log(&peer, &outcome);
        // A watch's "watching" was sent before its first state; anything else is answered now.
        if !(watching && answer == Answer::Watching) {
            connection.send(&answer)?;
        }
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
