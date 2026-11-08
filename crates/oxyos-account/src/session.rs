//! The session socket, `/run/oxyos/accountd/session` ([ADR 0020](../../docs/adr/0020-oxy-account-login.mdx),
//! decision 6, step 5): how a signed-in session's applications get the platform's Oxy session
//! (Oxy ADR 0058) without ever holding its credential.
//!
//! - **Transport:** `SOCK_SEQPACKET`, one JSON message per packet, as the service's own socket.
//!   Mode 0666: every peer is known by the kernel (`SO_PEERCRED` and `SO_PEERPIDFD`), never by
//!   what it sends. A peer that is neither of the two below is answered
//!   `{"refused": {"reason": "notPermitted"}}`, once its first message is read, and nothing else.
//! - **The registrar** is the session program the login program started: its uid is an enrolled
//!   account's user; the program its pidfd's process runs (`/proc/<pid>/exe`, the file itself, not
//!   a path) is `OXYOS_SESSION_PROGRAM`'s, owned by root and writable by no group or other; and its
//!   parent is a root process running `OXYOS_LOGIN_PROGRAM`'s file (the login program's worker,
//!   which starts the session with a clean environment). The session program run by the user
//!   itself (from a terminal, with `LD_PRELOAD`) is no registrar: its parent is the user's. It says what it
//!   launched, the launched process handed over as a pidfd (`SCM_RIGHTS`), which must be its own
//!   child and its user's:
//!
//!   ```text
//!   → {"present": {}}                                                       as it connects
//!   ← "accepted" | {"refused": {"reason": "notPermitted"}}, then closed
//!   → {"register": {"desktopEntryId": "so.oxy.dropsea", "system": true}}   + the child's pidfd
//!   ← "registered" | {"refused": {"reason"}}
//!   → {"check": {"password", "local": "accepted" | "rejected"}}          the lock's online check
//!   ← {"checked": {"result", "reason"?}}                                  for its own account
//!   ```
//!
//!   Accepted, its account's keychain is unlocked with that password; the registrar also asks
//!   AutoFill's requests and locks the keychain when the session locks (`crate::keychain`). The
//!   session's last registrar connection closed, its keychain locks.
//!
//!   A registration lives while its process does (the service watches the pidfd) and while the
//!   registrar's connection stays open: the session keeps one for its life, and closing it drops
//!   every registration made on it.
//!
//!   A registrar verified is kept ([`VERIFIED`]: its process by pid and start time, the file it
//!   runs and its parent's by device and inode), so the service started again (a package upgrade
//!   restarts it) still knows a session whose program's file the upgrade replaced: the session
//!   connects again and registers what still runs ([`RegistrarThread`]), and the service takes it
//!   if the kernel shows the same process running the same file under the same parent. Nothing the
//!   session says counts, and a boot keeps nothing.
//! - **An application** is a process whose pidfd is a live registration's, of the registrar's
//!   user. Anything else (another user, a process nobody registered, a child of a launched one)
//!   is refused. It may ask, any number of times on one connection:
//!
//!   ```text
//!   → "snapshot"                         ← {"snapshot": Snapshot}
//!   → "subscribe"                        ← {"snapshot": Snapshot}, now and on every change,
//!                                          until the client closes (nothing else is answered)
//!   → {"token": {"reason": "boot" | "preflight" | "response-401"}}
//!                                        ← {"token": {"accessToken", "expiresAt"}} | {"token": null}
//!   → {"remove": {}}                     ← "removed" | {"refused": {"reason"}}
//!   → {"erasePreview": {}}               ← {"erasePreview": {"available", "administrator",
//!                                            "online", "accounts": [{"name", "handle",
//!                                            "signsOutOnline"}]}}
//!   → {"eraseAll": {"password"}}         ← {"erasing": {"signedOut", "notSignedOut"}}, then the
//!                                          machine restarts | {"refused": {"reason",
//!                                          "retryAfter"?}}
//!   ```
//!
//!   `Snapshot` is the SDK's `OxyHostSnapshot`: `{"status": "signed_in" | "signed_out" |
//!   "unavailable", "revision", "account": {"id", "handle", "displayName"?, "avatar"?,
//!   "languages"?} | null, "online"}`, `languages` the account's locales, primary first, as the
//!   platform last read them online. A token is the application's own (Settings and the shell,
//!   registered as system applications, are OxyOS's; any other by its desktop entry id); null
//!   offline, signed out or refused. `remove` is Settings' alone, registered as a system application: decision 5's Remove
//!   of the peer's own account. So are `erasePreview` and `eraseAll`, erasing the machine
//!   ([ADR 0054](../../docs/adr/0054-recovery-and-erasing.mdx), [`erase`](crate::erase)): an
//!   erase is refused `notAdministrator`, `wrongPassword`, `throttled` (with `retryAfter`, in
//!   seconds), `unavailable` (no recovery) or `failed`, and a refusal changed nothing.
//!   The account's local sign-in (`crate::pin`): Settings and the shell's lock screen read it,
//!   Settings alone changes it, the person proved by the password or the PIN:
//!
//!   ```text
//!   → "localSignIn"                      ← {"localSignIn": {"pin": "none" | "set" | "locked",
//!                                                           "password": bool}}
//!   → {"setPin": {"pin", "password"? | "currentPin"?}}            ← {"localSignIn": …}
//!   → {"removePin": {"password"}}                                 ← {"localSignIn": …}
//!   → {"setPasswordSignIn": {"enabled", "password"? | "currentPin"?}}
//!                                                                 ← {"localSignIn": …}
//!   ```
//!
//!   refused `wrongPassword`, `wrongPin`, `throttled` (`retryAfter`), `pinLocked`,
//!   `unusablePin`, `passwordSignInOff`, `noPin`, `offline`, `secondFactorRequired` or `failed`;
//!   a refusal changed nothing. The keychain's
//!   requests (`crate::keychain`) are Settings' too; any other application is refused
//!   `notPermitted`.
//! - **Limits,** as the service's own socket: every peer waits at most [`IDLE`] for its next
//!   message but a registrar or a subscriber, at most [`MAX_REGISTRARS`] registrars and
//!   [`MAX_APPLICATION_CONNECTIONS`] application connections per user (so one user's applications
//!   never keep another's lock from its check),
//!   [`MAX_REGISTRATIONS`] registrations per registrar and [`MAX_SUBSCRIPTIONS`] subscriptions per
//!   registration. Every answer follows the message it answers, and no connection is closed with
//!   a message unread.

use crate::api::OxyApi;
use crate::erase::{ErasePreview, Eraser, Erasing, Refusal};
use crate::ipc::{Connection, MAX_MESSAGE};
use crate::keychain::passkey::{PasskeyAssertion, PasskeyCreate, PasskeyCreated, PasskeyGet};
use crate::keychain::{
    self, AuditView, CodeView, EnrollmentView, Field, ItemInput, ItemKind, ItemSummary, ItemView,
    JoinMethod, MemberView, SaveConfirmation, SaveLogin, Status, Suggestion,
};
use crate::pidfd::{self, Program};
use crate::platform::{self, Checked, Local, Platform, Snapshot, Token, TokenReason};
use crate::secret::Secret;
use crate::store::Enrollment;
use crate::users::Users;
use rustix::event::{poll, PollFd, PollFlags};
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Where the service listens for sessions and their applications.
pub const SOCKET: &str = "/run/oxyos/accountd/session";

/// The session program, unless `OXYOS_SESSION_PROGRAM` names another.
pub const SESSION_PROGRAM: &str = "/usr/bin/oxyos-session";

/// How long an application's connection waits for its next message.
pub const IDLE: Duration = Duration::from_secs(60);
/// Registrar connections of one user at once: the session keeps four (its launches', AutoFill's,
/// the save confirmations' and the passkey prompts'), and the lock's check makes one while it asks.
pub const MAX_REGISTRARS: usize = 6;
/// Application connections of one user at once.
pub const MAX_APPLICATION_CONNECTIONS: usize = 16;

/// The login program, unless `OXYOS_LOGIN_PROGRAM` names another: the registrar's parent.
pub const LOGIN_PROGRAM: &str = "/usr/libexec/oxyos/oxyos-login";
/// Registrations of one registrar at once.
pub const MAX_REGISTRATIONS: usize = 512;
/// Subscriptions of one registration at once.
pub const MAX_SUBSCRIPTIONS: usize = 4;
/// How long an application's `savePassword` waits for the person's answer in the session's sheet:
/// past it, nothing is saved and the sheet is withdrawn.
pub const SAVE_WAIT: Duration = Duration::from_secs(120);
/// Bytes of a username or a password an application asks to save.
pub const MAX_SAVED_USERNAME: usize = 512;
pub const MAX_SAVED_PASSWORD: usize = 1024;

const _: () = assert!(MAX_MESSAGE >= 4096);

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Request {
    /// The registrar presenting itself as its connection opens: answered `"accepted"` if the
    /// kernel's word on it says it is one, refused `notPermitted` (and closed) if not.
    Present {},
    #[serde(rename_all = "camelCase")]
    Register {
        desktop_entry_id: String,
        #[serde(default)]
        system: bool,
    },
    Check {
        password: Secret,
        local: Local,
    },
    Snapshot,
    Subscribe,
    Token {
        reason: TokenReason,
    },
    Remove {},
    // The keychain (ADR 0050 step 2, `crate::keychain`): the registrar's.
    #[serde(rename_all = "camelCase")]
    AutofillSuggest {
        desktop_entry_id: String,
        field: Field,
    },
    #[serde(rename_all = "camelCase")]
    AutofillFill {
        desktop_entry_id: String,
        #[serde(rename = "ref")]
        reference: String,
        field: Field,
    },
    AutofillGenerate {},
    KeychainLock {},
    // Settings'.
    KeychainStatus {},
    KeychainCreate {},
    #[serde(rename_all = "camelCase")]
    KeychainConfirmRecoveryKey {
        recovery_key: Secret,
    },
    #[serde(rename_all = "camelCase")]
    KeychainJoin {
        method: JoinMethod,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recovery_key: Option<Secret>,
    },
    KeychainEnrollments {},
    #[serde(rename_all = "camelCase")]
    KeychainApprove {
        enrollment_id: String,
        #[serde(default)]
        confirm: bool,
    },
    #[serde(rename_all = "camelCase")]
    KeychainDeny {
        enrollment_id: String,
    },
    KeychainMembers {},
    #[serde(rename_all = "camelCase")]
    KeychainRemoveMember {
        member_id: String,
    },
    KeychainVerify {
        password: Secret,
    },
    KeychainList {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query: Option<Secret>,
        #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
        kind: Option<ItemKind>,
    },
    #[serde(rename_all = "camelCase")]
    KeychainReveal {
        item_id: String,
    },
    KeychainPut {
        item: ItemInput,
    },
    #[serde(rename_all = "camelCase")]
    KeychainDelete {
        item_id: String,
    },
    KeychainAudit {},
    KeychainRekey {
        previous: Secret,
        current: Secret,
    },
    KeychainLeave {},
    /// A strong password (ADR 0050 decision 8's generator), for a new or edited item.
    KeychainGenerate {},
    /// An item's verification code now.
    #[serde(rename_all = "camelCase")]
    KeychainCode {
        item_id: String,
    },
    // Saving a credential (ADR 0050 decision 5, `crate::keychain`): an application asks, the
    // registrar's connection that takes confirmations (`confirmSaves`) asks the person and answers
    // each (`saveDecision`).
    SavePassword {
        username: Secret,
        password: Secret,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        domain: Option<String>,
    },
    ConfirmSaves {},
    SaveDecision {
        id: String,
        save: bool,
    },
    // Passkeys (ADR 0050 step 5, `crate::keychain::passkey`): any registered application's.
    PasskeyCreate(PasskeyCreate),
    PasskeyGet(PasskeyGet),
    // The registrar's prompts connection.
    PasskeyPrompts {},
    PasskeyConfirm {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        choice: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<Secret>,
    },
    PasskeyCancel {
        id: String,
    },
    ErasePreview {},
    EraseAll {
        password: Secret,
    },
    // The local sign-in (`crate::pin`): Settings' and the shell's to read, Settings' to change.
    LocalSignIn,
    #[serde(rename_all = "camelCase")]
    SetPin {
        pin: Secret,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<Secret>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        current_pin: Option<Secret>,
    },
    RemovePin {
        password: Secret,
    },
    #[serde(rename_all = "camelCase")]
    SetPasswordSignIn {
        enabled: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<Secret>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        current_pin: Option<Secret>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    NotPermitted,
    InvalidRequest,
    TooMany,
    Failed,
    // The keychain's.
    /// The keychain is locked (the session is, or was never unlocked since the service started).
    Locked,
    /// Oxy cannot be reached.
    Offline,
    /// Oxy refused, or answered what does not verify: nothing of it was kept.
    Oxy,
    /// The account has no vault, or the machine is no member of it.
    NotMember,
    /// The account already has a vault (join it), or this machine is a member already.
    VaultExists,
    /// Another device removed this machine, or the vault was reset.
    Removed,
    /// The keybag is sealed under a password the machine no longer has (`keychainRekey`).
    NeedsPreviousPassword,
    /// Settings must verify the person first (`keychainVerify`).
    VerificationRequired,
    WrongPassword,
    /// Too many wrong passwords: try again later (an erase says when, `retryAfter`).
    Throttled,
    /// Not the recovery key shown, or not the vault's.
    RecoveryKeyMismatch,
    /// No such item, enrollment, member or suggestion (or its `ref` died with a lock).
    NotFound,
    /// The enrollment was denied.
    Denied,
    /// The enrollment's ten minutes are over.
    Expired,
    /// Another device wrote first: try again.
    Conflict,
    /// An erase asked by an account that is not one of the machine's administrators.
    NotAdministrator,
    /// An erase of a machine without a recovery; a save with no session to ask the person in.
    Unavailable,
    // The local sign-in's (`crate::pin`).
    WrongPin,
    /// Too many wrong PINs: the password, or Oxy online, first.
    PinLocked,
    /// Not 4 to 16 digits, or the password itself.
    UnusablePin,
    /// The password does not sign in on this computer.
    PasswordSignInOff,
    /// The password cannot be turned off without a PIN.
    NoPin,
    /// The account has an authenticator: set it up again at the login screen.
    SecondFactorRequired,
    // Passkeys', as WebAuthn's errors (`crate::keychain::passkey`).
    /// `NotAllowedError`: cancelled, not verified, timed out, no passkey, or a relying party the
    /// application may not use, all alike.
    NotAllowed,
    /// `InvalidStateError`: an excluded credential is in the keychain.
    InvalidState,
    /// `NotSupportedError`: no algorithm OxyOS makes (ES256).
    NotSupported,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Answer {
    Accepted,
    Registered,
    Refused {
        reason: Reason,
        /// Seconds until another attempt may be made (`throttled`).
        #[serde(
            default,
            rename = "retryAfter",
            skip_serializing_if = "Option::is_none"
        )]
        retry_after: Option<u64>,
    },
    Checked(Checked),
    Snapshot(Snapshot),
    Token(Option<Token>),
    Removed,
    // The keychain's (`crate::keychain`).
    AutofillSuggestions(Vec<Suggestion>),
    AutofillCredential {
        username: Secret,
        password: Secret,
    },
    AutofillCode {
        code: Secret,
    },
    GeneratedPassword(Secret),
    KeychainLocked,
    KeychainStatus(Status),
    #[serde(rename_all = "camelCase")]
    KeychainRecoveryKey {
        recovery_key: Secret,
    },
    KeychainEnrollment(EnrollmentView),
    KeychainEnrollments(Vec<EnrollmentView>),
    KeychainApproved,
    KeychainDenied,
    KeychainMembers(Vec<MemberView>),
    KeychainMemberRemoved,
    KeychainVerified,
    KeychainItems(Vec<ItemSummary>),
    KeychainItem(ItemView),
    #[serde(rename_all = "camelCase")]
    KeychainSaved {
        item_id: String,
    },
    KeychainDeleted,
    KeychainAudit(AuditView),
    KeychainLeft,
    KeychainCode(CodeView),
    // Saving a credential (`crate::keychain`): the application's answers, and what the
    // confirmations' connection is told.
    Saved {},
    NotSaved {},
    ConfirmingSaves,
    ConfirmSave(SaveConfirmation),
    SaveWithdrawn {
        id: String,
    },
    PasskeyCreated(PasskeyCreated),
    PasskeyAssertion(PasskeyAssertion),
    PasskeyPromptsOpen,
    ErasePreview(ErasePreview),
    Erasing(Erasing),
    LocalSignIn(platform::LocalView),
}

impl Answer {
    pub fn refused(reason: Reason) -> Self {
        Answer::Refused {
            reason,
            retry_after: None,
        }
    }
}

impl From<platform::LocalRefusal> for Answer {
    fn from(refusal: platform::LocalRefusal) -> Self {
        use platform::LocalRefusal as R;
        let reason = match refusal {
            R::WrongPassword => Reason::WrongPassword,
            R::WrongPin => Reason::WrongPin,
            R::Throttled(seconds) => {
                return Answer::Refused {
                    reason: Reason::Throttled,
                    retry_after: Some(seconds),
                }
            }
            R::PinLocked => Reason::PinLocked,
            R::UnusablePin => Reason::UnusablePin,
            R::PasswordSignInOff => Reason::PasswordSignInOff,
            R::NoPin => Reason::NoPin,
            R::Offline => Reason::Offline,
            R::SecondFactorRequired => Reason::SecondFactorRequired,
            R::Failed => Reason::Failed,
        };
        Answer::refused(reason)
    }
}

impl From<Refusal> for Answer {
    fn from(refusal: Refusal) -> Self {
        let (reason, retry_after) = match refusal {
            Refusal::NotAdministrator => (Reason::NotAdministrator, None),
            Refusal::WrongPassword => (Reason::WrongPassword, None),
            Refusal::Throttled { retry_after } => (Reason::Throttled, Some(retry_after)),
            Refusal::Unavailable => (Reason::Unavailable, None),
            Refusal::Failed => (Reason::Failed, None),
        };
        Answer::Refused {
            reason,
            retry_after,
        }
    }
}

/// Who may register: the session program, by its file.
#[derive(Clone, Debug)]
pub struct Registrars {
    pub program: PathBuf,
    /// The program that starts it, whose process is the registrar's parent.
    pub parent: PathBuf,
    /// Both programs' owner and the parent's user: root (the tests', their own user).
    pub owner: u32,
    /// Where the service keeps the registrars it verified this boot ([`VERIFIED`]), so that one
    /// still running when the service starts again is known though an upgrade replaced its
    /// program's file since; None: only the files as they are now are taken.
    pub verified: Option<PathBuf>,
}

/// Where the service keeps the registrars it verified, one file per process: the service's own,
/// 0700, under `/run` (so a boot starts with none), and kept when the service restarts
/// (`RuntimeDirectoryPreserve=yes`).
pub const VERIFIED: &str = "/run/oxyos/accountd/registrars";

/// A registrar the service verified, as the kernel showed it then: the process (by its pid and
/// start time, one process for the whole boot), the file it ran (by device and inode: a file
/// replaced since is still held by the process running it, so its inode is never another file's
/// meanwhile), and the same of its parent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Verified {
    uid: u32,
    started: u64,
    device: u64,
    inode: u64,
    parent: i32,
    parent_started: u64,
    parent_device: u64,
    parent_inode: u64,
}

/// How large a kept registrar's file may be.
const VERIFIED_LIMIT: u64 = 4096;

impl Registrars {
    pub fn new(program: PathBuf, parent: PathBuf) -> Self {
        Self {
            program,
            parent,
            owner: 0,
            verified: None,
        }
    }

    /// The registrars this boot's service verified are kept in `directory` ([`VERIFIED`]).
    pub fn remembering(mut self, directory: PathBuf) -> Self {
        self.verified = Some(directory);
        self
    }

    /// Whether `pidfd`'s process runs the session program and its parent, of the owner's user,
    /// the login program; judged while it lives. A process that does is kept ([`Verified`]); one
    /// whose program's file an upgrade replaced since (it runs the file it was verified running,
    /// no longer at the program's path) is known by what was kept of it: the same process, the
    /// same file, the same parent running the same file, every one read again from the kernel.
    /// Nothing the peer says counts.
    fn verify(&self, pidfd: &OwnedFd) -> bool {
        let program = Program::at(&self.program).ok();
        let parent = Program::at(&self.parent).ok();
        let observed = pidfd::while_alive(pidfd, |pid| {
            let running = Program::of_process(pid)?;
            let status = pidfd::status(pid)?;
            let started_by = Program::of_process(status.parent)?;
            let starter = pidfd::status(status.parent)?;
            let seen = Verified {
                uid: status.uid,
                started: pidfd::started(pid)?,
                device: running.device,
                inode: running.inode,
                parent: status.parent,
                parent_started: pidfd::started(status.parent)?,
                parent_device: started_by.device,
                parent_inode: started_by.inode,
            };
            // Root's files, written by no one else, the parent root's: what was verified once is
            // still that.
            let kept = running.is(&running, self.owner)
                && started_by.is(&started_by, self.owner)
                && starter.uid == self.owner;
            let now = kept
                && program.is_some_and(|program| running.is(&program, self.owner))
                && parent.is_some_and(|parent| started_by.is(&parent, self.owner));
            Ok((seen, kept, now))
        });
        let Some((pid, (seen, kept, now))) = observed else {
            return false;
        };
        if now {
            self.remember(pid, &seen);
            return true;
        }
        kept && self.recall(pid) == Some(seen)
    }

    /// Keeps `seen`, the verified registrar `pid`, and forgets those whose process is gone.
    fn remember(&self, pid: i32, seen: &Verified) {
        let Some(directory) = &self.verified else {
            return;
        };
        if let Err(error) = write_verified(directory, pid, seen) {
            eprintln!(
                "oxyos-accountd: {}: a registrar not kept: {error}",
                directory.display()
            );
        }
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(other) = name.to_str().and_then(|name| name.parse::<i32>().ok()) else {
                continue;
            };
            if other == pid {
                continue;
            }
            let gone = match self.recall(other) {
                Some(kept) => pidfd::started(other).ok() != Some(kept.started),
                None => true,
            };
            if gone {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    /// What was kept of the registrar `pid`, if the service kept it: a file of the service's own
    /// user in its own directory, written by no one else.
    fn recall(&self, pid: i32) -> Option<Verified> {
        use std::io::Read;
        use std::os::unix::fs::MetadataExt;
        let directory = self.verified.as_ref()?;
        let me = rustix::process::geteuid().as_raw();
        let ours =
            |metadata: &std::fs::Metadata| metadata.uid() == me && metadata.mode() & 0o077 == 0;
        let dir = std::fs::symlink_metadata(directory).ok()?;
        if !dir.is_dir() || !ours(&dir) {
            return None;
        }
        let path = directory.join(pid.to_string());
        let file = std::fs::symlink_metadata(&path).ok()?;
        if !file.is_file() || !ours(&file) {
            return None;
        }
        let mut text = String::new();
        std::fs::File::open(&path)
            .ok()?
            .take(VERIFIED_LIMIT)
            .read_to_string(&mut text)
            .ok()?;
        serde_json::from_str(&text).ok()
    }
}

/// `seen` written whole for `pid` in `directory`: a file made beside it, then renamed over it.
fn write_verified(directory: &Path, pid: i32, seen: &Verified) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let text = serde_json::to_vec(seen).map_err(io::Error::other)?;
    let making = directory.join(format!(".{pid}.new"));
    let _ = std::fs::remove_file(&making);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&making)?;
    file.write_all(&text)?;
    drop(file);
    std::fs::rename(&making, directory.join(pid.to_string()))
}

/// A process the session launched, while it runs and its registrar's connection is open.
pub struct Registration {
    pub desktop_entry_id: String,
    pub system: bool,
    pub uid: u32,
    pub pid: i32,
    pidfd: OwnedFd,
    registrar: u64,
    /// Written to once the registration has ended, for its subscribers.
    ended: OwnedFd,
    subscriptions: Mutex<usize>,
}

impl Registration {
    /// Whether its process is still the one registered.
    fn alive(&self) -> bool {
        matches!(pidfd::pid(self.pidfd.as_fd()), Ok(Some(pid)) if pid == self.pid)
    }
}

/// What a peer of the session socket is.
pub enum Kind {
    Registrar {
        id: u64,
        enrollment: Box<Enrollment>,
        pid: i32,
    },
    Application(Arc<Registration>),
    Refused,
}

/// The session socket's side of the service.
pub struct Sessions<A, U> {
    platform: Arc<Platform<A, U>>,
    eraser: Eraser,
    registrars: Registrars,
    registrations: Mutex<Vec<Arc<Registration>>>,
    /// Registrar connections open, by uid.
    open_registrars: Mutex<Vec<u32>>,
    /// Application connections open, by uid.
    open_applications: Mutex<Vec<u32>>,
    next: AtomicU64,
    saves: Saves,
    /// [`SAVE_WAIT`], in milliseconds (shorter in the tests).
    save_wait: AtomicU64,
}

/// Saving a credential (ADR 0050 decision 5): the session's connection that asks the person, one
/// per user, and the applications' requests waiting for their answer. Locked in this order when
/// both are: `pending`, then `confirmers`.
#[derive(Default)]
struct Saves {
    confirmers: Mutex<std::collections::HashMap<u32, Confirmer>>,
    pending: Mutex<std::collections::HashMap<String, PendingSave>>,
    next: AtomicU64,
}

/// A registrar connection that takes save confirmations: what it is to be told, and what wakes it.
struct Confirmer {
    registrar: u64,
    outbox: Arc<Mutex<std::collections::VecDeque<Answer>>>,
    wake: Arc<OwnedFd>,
}

impl Confirmer {
    fn tell(&self, answer: Answer) {
        self.outbox
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push_back(answer);
        let _ = rustix::io::write(&*self.wake, &1u64.to_ne_bytes());
    }
}

/// An application's request to save, waiting for the person's answer.
struct PendingSave {
    uid: u32,
    desktop_entry_id: String,
    /// The confirmations' connection it was asked on: only it answers.
    registrar: u64,
    decided: std::sync::mpsc::SyncSender<bool>,
}

/// Takes one of `uid`'s places among `open`, if it has fewer than `limit`.
fn take(open: &Mutex<Vec<u32>>, uid: u32, limit: usize) -> bool {
    let mut open = open.lock().unwrap_or_else(|p| p.into_inner());
    if open.iter().filter(|held| **held == uid).count() >= limit {
        return false;
    }
    open.push(uid);
    true
}

fn give_back(open: &Mutex<Vec<u32>>, uid: u32) {
    let mut open = open.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(index) = open.iter().position(|held| *held == uid) {
        open.remove(index);
    }
}

fn eventfd() -> io::Result<OwnedFd> {
    use rustix::event::EventfdFlags;
    Ok(rustix::event::eventfd(
        0,
        EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK,
    )?)
}

fn drain(fd: &OwnedFd) {
    let mut count = [0u8; 8];
    let _ = rustix::io::read(fd, &mut count);
}

/// What a connection's peer did, for the service's log: never a secret.
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    Registered {
        id: String,
        pid: i32,
    },
    RegistrationRefused {
        id: String,
        reason: Reason,
    },
    Checked(Checked),
    Removed,
    /// Settings asked what erasing would do.
    ErasePreviewed,
    /// The erase is armed: the machine restarts. How many accounts were signed out online, and
    /// how many not.
    Erasing {
        signed_out: usize,
        not_signed_out: usize,
    },
    EraseRefused(Reason),
    /// An application's login saved once the person said yes in the session's sheet (the
    /// application's desktop entry id; never what was saved).
    Saved {
        id: String,
    },
    /// The person said no, or did not answer in time.
    NotSaved {
        id: String,
    },
    /// An application's request to save refused, and why.
    SaveRefused {
        id: String,
        reason: Reason,
    },
    Refused(Reason),
    /// Settings changed the local sign-in (`setPin`, `removePin`, `passwordOn`, `passwordOff`),
    /// or was refused.
    LocalChanged {
        change: &'static str,
        refused: Option<Reason>,
    },
}

impl<A: OxyApi + Send + Sync + 'static, U: Users + Send + Sync + 'static> Sessions<A, U> {
    pub fn new(platform: Arc<Platform<A, U>>, registrars: Registrars, eraser: Eraser) -> Self {
        Self {
            platform,
            eraser,
            registrars,
            registrations: Mutex::new(Vec::new()),
            open_registrars: Mutex::new(Vec::new()),
            open_applications: Mutex::new(Vec::new()),
            next: AtomicU64::new(1),
            saves: Saves::default(),
            save_wait: AtomicU64::new(SAVE_WAIT.as_millis() as u64),
        }
    }

    /// How long a save waits for the person's answer ([`SAVE_WAIT`]).
    pub fn set_save_wait(&self, wait: Duration) {
        self.save_wait.store(
            u64::try_from(wait.as_millis()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
    }

    pub fn platform(&self) -> &Arc<Platform<A, U>> {
        &self.platform
    }

    /// The live registrations (for the tests and the log).
    pub fn registered(&self) -> Vec<(String, i32)> {
        self.registrations
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|registration| (registration.desktop_entry_id.clone(), registration.pid))
            .collect()
    }

    /// What the peer of `connection` is, from the kernel's word on it.
    pub fn classify(&self, connection: &Connection) -> Kind {
        let (Ok(peer), Ok(pidfd)) = (connection.peer(), connection.peer_pidfd()) else {
            return Kind::Refused;
        };
        let Some(peer_pid) = pidfd::pid(pidfd.as_fd()).ok().flatten() else {
            return Kind::Refused;
        };
        // A registered application: its pidfd's pid is a live registration's, of its user. Both
        // pids are read again after: two live processes never share one.
        let registration = {
            let registrations = self.registrations.lock().unwrap_or_else(|p| p.into_inner());
            registrations
                .iter()
                .find(|registration| registration.pid == peer_pid && registration.uid == peer.uid)
                .cloned()
        };
        if let Some(registration) = registration {
            if registration.alive()
                && matches!(pidfd::pid(pidfd.as_fd()), Ok(Some(pid)) if pid == peer_pid)
                && take(
                    &self.open_applications,
                    peer.uid,
                    MAX_APPLICATION_CONNECTIONS,
                )
            {
                return Kind::Application(registration);
            }
            return Kind::Refused;
        }
        // The registrar: an enrolled account's user running the session program.
        let Ok(Some(enrollment)) = self.platform.enrollment_of(peer.uid) else {
            return Kind::Refused;
        };
        if !self.registrars.verify(&pidfd) {
            return Kind::Refused;
        }
        if !take(&self.open_registrars, peer.uid, MAX_REGISTRARS) {
            return Kind::Refused;
        }
        Kind::Registrar {
            id: self.next.fetch_add(1, Ordering::SeqCst),
            enrollment: Box::new(enrollment),
            pid: peer_pid,
        }
    }

    /// Serves `connection` as `kind` says, to its end. Each event is handed to `log`.
    pub fn serve(&self, connection: &Connection, kind: Kind, mut log: impl FnMut(&Event)) {
        match kind {
            Kind::Refused => {
                log(&Event::Refused(Reason::NotPermitted));
                let _ = refuse(connection);
            }
            Kind::Registrar {
                id,
                enrollment,
                pid,
            } => {
                let uid = enrollment.uid;
                if let Some(keychains) = self.platform.keychains() {
                    keychains.session_opened(uid, enrollment.user.as_str());
                }
                let served = self.serve_registrar(connection, id, &enrollment, pid, &mut log);
                self.drop_registrar(id);
                self.release_registrar(uid);
                // The session's last connection gone, its keychain locks (ADR 0050 decision 2).
                if let Some(keychains) = self.platform.keychains() {
                    let remaining = self
                        .open_registrars
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .iter()
                        .filter(|held| **held == uid)
                        .count();
                    keychains.session_closed(uid, remaining);
                }
                if let Err(error) = served {
                    eprintln!("oxyos-accountd: a session's connection ended: {error}");
                }
            }
            Kind::Application(registration) => {
                if let Err(error) = self.serve_application(connection, &registration, &mut log) {
                    if error.kind() != io::ErrorKind::WouldBlock {
                        eprintln!(
                            "oxyos-accountd: {}'s connection ended: {error}",
                            registration.desktop_entry_id
                        );
                    }
                }
                give_back(&self.open_applications, registration.uid);
            }
        }
    }

    /// A peer classified and then not served (too many at once): what classifying it took is
    /// given back. Its connection is closed unanswered.
    pub fn unserved(&self, kind: Kind) {
        match kind {
            Kind::Registrar { enrollment, .. } => self.release_registrar(enrollment.uid),
            Kind::Application(registration) => give_back(&self.open_applications, registration.uid),
            Kind::Refused => {}
        }
    }

    fn release_registrar(&self, uid: u32) {
        give_back(&self.open_registrars, uid);
    }

    fn drop_registrar(&self, registrar: u64) {
        let mut registrations = self.registrations.lock().unwrap_or_else(|p| p.into_inner());
        registrations.retain(|registration| {
            let keep = registration.registrar != registrar;
            if !keep {
                let _ = rustix::io::write(&registration.ended, &1u64.to_ne_bytes());
            }
            keep
        });
    }

    fn drop_registration(&self, registration: &Arc<Registration>) {
        let mut registrations = self.registrations.lock().unwrap_or_else(|p| p.into_inner());
        registrations.retain(|held| !Arc::ptr_eq(held, registration));
        let _ = rustix::io::write(&registration.ended, &1u64.to_ne_bytes());
    }

    fn serve_registrar(
        &self,
        connection: &Connection,
        id: u64,
        enrollment: &Enrollment,
        registrar_pid: i32,
        log: &mut impl FnMut(&Event),
    ) -> io::Result<()> {
        connection.set_receive_timeout(None)?;
        loop {
            // The connection, and each of its registrations' processes: one that exits is
            // forgotten.
            let mine: Vec<Arc<Registration>> = self
                .registrations
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .iter()
                .filter(|registration| registration.registrar == id)
                .cloned()
                .collect();
            let mut fds = vec![PollFd::new(connection, PollFlags::IN)];
            fds.extend(
                mine.iter()
                    .map(|registration| PollFd::new(&registration.pidfd, PollFlags::IN)),
            );
            match poll(&mut fds, None) {
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => return Err(error.into()),
            }
            let message_waiting = !fds[0].revents().is_empty();
            let exited: Vec<bool> = fds[1..].iter().map(|fd| !fd.revents().is_empty()).collect();
            drop(fds);
            for (registration, exited) in mine.iter().zip(exited) {
                if exited {
                    self.drop_registration(registration);
                }
            }
            if !message_waiting {
                continue;
            }
            let Some((request, fds)) = connection.receive_with::<Request>()? else {
                return Ok(());
            };
            let answer = match request {
                Some(Request::Present {}) => Answer::Accepted,
                Some(Request::Register {
                    desktop_entry_id,
                    system,
                }) => {
                    let answer = self.register(
                        id,
                        enrollment.uid,
                        registrar_pid,
                        &desktop_entry_id,
                        system,
                        fds,
                    );
                    log(&match &answer {
                        Ok(pid) => Event::Registered {
                            id: desktop_entry_id,
                            pid: *pid,
                        },
                        Err(reason) => Event::RegistrationRefused {
                            id: desktop_entry_id,
                            reason: *reason,
                        },
                    });
                    match answer {
                        Ok(_) => Answer::Registered,
                        Err(reason) => Answer::refused(reason),
                    }
                }
                Some(Request::Check { password, local }) => {
                    let unlocking = Secret::new(password.expose());
                    let checked = self
                        .platform
                        .check(enrollment.user.as_str(), password, local);
                    log(&Event::Checked(checked));
                    connection.send(&Answer::Checked(checked))?;
                    // Accepted, the password opens the account's keychain (ADR 0050 decision 2),
                    // after the lock has its answer: the keybag's Argon2id never delays it.
                    if checked.result == platform::Verdict::Accepted {
                        if let Some(keychains) = self.platform.keychains() {
                            keychains.unlock(enrollment.uid, enrollment.user.as_str(), &unlocking);
                        }
                    }
                    continue;
                }
                // The session's prompts connection: from now on, the passkey sheets to show.
                Some(Request::PasskeyPrompts {}) => {
                    let Some(keychains) = self.platform.keychains() else {
                        connection.send(&Answer::refused(Reason::Locked))?;
                        continue;
                    };
                    return keychains.serve_prompts(
                        enrollment.uid,
                        enrollment.user.as_str(),
                        connection,
                    );
                }
                Some(request) if keychain::is_registrar_request(&request) => {
                    match self.platform.keychains() {
                        Some(keychains) => {
                            keychains.registrar(enrollment.uid, enrollment.user.as_str(), request)
                        }
                        None => Answer::refused(Reason::Locked),
                    }
                }
                // From here on this connection takes the save confirmations of its user's
                // applications (ADR 0050 decision 5), and does nothing else: one that registered
                // launches keeps doing that alone.
                Some(Request::ConfirmSaves {}) => {
                    if !mine.is_empty() {
                        Answer::refused(Reason::InvalidRequest)
                    } else {
                        return self.confirm_saves(connection, id, enrollment.uid);
                    }
                }
                Some(_) => Answer::refused(Reason::NotPermitted),
                None => Answer::refused(Reason::InvalidRequest),
            };
            connection.send(&answer)?;
        }
    }

    /// The registrar connection `registrar` takes `uid`'s save confirmations until it closes:
    /// each application's `savePassword` is told it as `{"confirmSave": …}` (never the password),
    /// a request that ended unanswered as `{"saveWithdrawn": {"id"}}`, and it answers each
    /// `{"saveDecision": {"id", "save"}}`. A user has one at a time. Closing it answers every
    /// request it was asked `notSaved`.
    fn confirm_saves(&self, connection: &Connection, registrar: u64, uid: u32) -> io::Result<()> {
        let wake = Arc::new(eventfd()?);
        let outbox = Arc::new(Mutex::new(std::collections::VecDeque::new()));
        {
            let mut confirmers = self
                .saves
                .confirmers
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if confirmers.contains_key(&uid) {
                drop(confirmers);
                return connection.send(&Answer::refused(Reason::TooMany));
            }
            confirmers.insert(
                uid,
                Confirmer {
                    registrar,
                    outbox: outbox.clone(),
                    wake: wake.clone(),
                },
            );
        }
        let served = (|| -> io::Result<()> {
            connection.send(&Answer::ConfirmingSaves)?;
            loop {
                let mut fds = [
                    PollFd::new(connection, PollFlags::IN),
                    PollFd::new(&*wake, PollFlags::IN),
                ];
                match poll(&mut fds, None) {
                    Ok(_) => {}
                    Err(rustix::io::Errno::INTR) => continue,
                    Err(error) => return Err(error.into()),
                }
                let (message, woken) = (!fds[0].revents().is_empty(), !fds[1].revents().is_empty());
                if woken {
                    drain(&wake);
                    loop {
                        let next = outbox.lock().unwrap_or_else(|p| p.into_inner()).pop_front();
                        let Some(told) = next else { break };
                        connection.send(&told)?;
                    }
                }
                if !message {
                    continue;
                }
                match connection.receive::<Request>()? {
                    None => return Ok(()),
                    Some(Some(Request::SaveDecision { id, save })) => {
                        let pending = self.saves.pending.lock().unwrap_or_else(|p| p.into_inner());
                        // Only the request this connection was asked, of its own user.
                        if let Some(waiting) = pending
                            .get(&id)
                            .filter(|waiting| waiting.uid == uid && waiting.registrar == registrar)
                        {
                            let _ = waiting.decided.try_send(save);
                        }
                    }
                    Some(_) => connection.send(&Answer::refused(Reason::InvalidRequest))?,
                }
            }
        })();
        {
            let mut confirmers = self
                .saves
                .confirmers
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if confirmers
                .get(&uid)
                .is_some_and(|confirmer| confirmer.registrar == registrar)
            {
                confirmers.remove(&uid);
            }
        }
        // Nobody is left to answer what it was asked: not saved.
        for waiting in self
            .saves
            .pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .filter(|waiting| waiting.registrar == registrar)
        {
            let _ = waiting.decided.try_send(false);
        }
        served
    }

    /// An application's request to save a login it has just used (ADR 0050 decision 5): never on
    /// its word. The keychain must be unlocked and a vault's member; the session's sheet asks the
    /// person, naming the application and the username; only their yes, within [`SAVE_WAIT`],
    /// saves it, associated with the application's desktop entry id (and `domain`'s URL). One
    /// request of an application waits at a time.
    fn save_password(
        &self,
        registration: &Registration,
        username: Secret,
        password: Secret,
        domain: Option<String>,
        log: &mut impl FnMut(&Event),
    ) -> Answer {
        let uid = registration.uid;
        let app = registration.desktop_entry_id.as_str();
        let domain = match domain.as_deref().map(keychain::saved_domain) {
            None => None,
            Some(Some(domain)) => Some(domain),
            Some(None) => return Answer::refused(Reason::InvalidRequest),
        };
        if !keychain::savable_text(username.expose(), MAX_SAVED_USERNAME)
            || password.is_empty()
            || password.expose().len() > MAX_SAVED_PASSWORD
        {
            return Answer::refused(Reason::InvalidRequest);
        }
        let Some(keychains) = self.platform.keychains() else {
            return Answer::refused(Reason::Locked);
        };
        let Ok(Some(enrollment)) = self.platform.enrollment_of(uid) else {
            return Answer::refused(Reason::Failed);
        };
        let user = enrollment.user.as_str();
        if let Err(reason) = keychains.may_save(uid, user) {
            log(&Event::SaveRefused {
                id: app.to_owned(),
                reason,
            });
            return Answer::refused(reason);
        }
        let (id, decided, registrar) = {
            let mut pending = self.saves.pending.lock().unwrap_or_else(|p| p.into_inner());
            if pending
                .values()
                .any(|waiting| waiting.uid == uid && waiting.desktop_entry_id == app)
            {
                log(&Event::SaveRefused {
                    id: app.to_owned(),
                    reason: Reason::TooMany,
                });
                return Answer::refused(Reason::TooMany);
            }
            let confirmers = self
                .saves
                .confirmers
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            let Some(confirmer) = confirmers.get(&uid) else {
                log(&Event::SaveRefused {
                    id: app.to_owned(),
                    reason: Reason::Unavailable,
                });
                return Answer::refused(Reason::Unavailable);
            };
            let id = self.saves.next.fetch_add(1, Ordering::SeqCst).to_string();
            let (decide, decided) = std::sync::mpsc::sync_channel(1);
            pending.insert(
                id.clone(),
                PendingSave {
                    uid,
                    desktop_entry_id: app.to_owned(),
                    registrar: confirmer.registrar,
                    decided: decide,
                },
            );
            confirmer.tell(Answer::ConfirmSave(SaveConfirmation {
                id: id.clone(),
                desktop_entry_id: app.to_owned(),
                username: Secret::new(username.expose()),
                domain: domain.clone(),
            }));
            (id, decided, confirmer.registrar)
        };
        let decision = decided.recv_timeout(Duration::from_millis(
            self.save_wait.load(Ordering::Relaxed),
        ));
        self.saves
            .pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&id);
        let save = match decision {
            Ok(save) => save,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // The sheet goes: nothing will be saved for it.
                if let Some(confirmer) = self
                    .saves
                    .confirmers
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .get(&uid)
                    .filter(|confirmer| confirmer.registrar == registrar)
                {
                    confirmer.tell(Answer::SaveWithdrawn { id });
                }
                false
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => false,
        };
        if !save {
            log(&Event::NotSaved { id: app.to_owned() });
            return Answer::NotSaved {};
        }
        let login = SaveLogin {
            desktop_entry_id: app.to_owned(),
            username,
            password,
            domain,
        };
        match keychains.save(uid, user, &login) {
            Ok(()) => {
                log(&Event::Saved { id: app.to_owned() });
                Answer::Saved {}
            }
            Err(reason) => {
                log(&Event::SaveRefused {
                    id: app.to_owned(),
                    reason,
                });
                Answer::refused(reason)
            }
        }
    }

    /// Registers the process `fds` hands over, if it is the registrar's own child, of its user.
    fn register(
        &self,
        registrar: u64,
        uid: u32,
        registrar_pid: i32,
        desktop_entry_id: &str,
        system: bool,
        mut fds: Vec<OwnedFd>,
    ) -> Result<i32, Reason> {
        if fds.len() != 1 || !valid_entry_id(desktop_entry_id) {
            return Err(Reason::InvalidRequest);
        }
        let pidfd = fds.remove(0);
        let (pid, status) =
            pidfd::while_alive(&pidfd, pidfd::status).ok_or(Reason::InvalidRequest)?;
        if status.parent != registrar_pid || status.uid != uid {
            return Err(Reason::NotPermitted);
        }
        let mut registrations = self.registrations.lock().unwrap_or_else(|p| p.into_inner());
        // A pid registered again (its earlier process gone) is the new one's.
        registrations.retain(|held| {
            let keep = !(held.pid == pid && held.uid == uid);
            if !keep {
                let _ = rustix::io::write(&held.ended, &1u64.to_ne_bytes());
            }
            keep
        });
        if registrations
            .iter()
            .filter(|held| held.registrar == registrar)
            .count()
            >= MAX_REGISTRATIONS
        {
            return Err(Reason::TooMany);
        }
        let ended = eventfd().map_err(|_| Reason::Failed)?;
        registrations.push(Arc::new(Registration {
            desktop_entry_id: desktop_entry_id.to_owned(),
            system,
            uid,
            pid,
            pidfd,
            registrar,
            ended,
            subscriptions: Mutex::new(0),
        }));
        Ok(pid)
    }

    fn live(&self, registration: &Arc<Registration>) -> bool {
        let held = self
            .registrations
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .any(|held| Arc::ptr_eq(held, registration));
        held && registration.alive()
    }

    fn serve_application(
        &self,
        connection: &Connection,
        registration: &Arc<Registration>,
        log: &mut impl FnMut(&Event),
    ) -> io::Result<()> {
        connection.set_receive_timeout(Some(IDLE))?;
        let uid = registration.uid;
        loop {
            let Some(request) = connection.receive::<Request>()? else {
                return Ok(());
            };
            if !self.live(registration) {
                return connection.send(&Answer::refused(Reason::NotPermitted));
            }
            let answer = match request {
                Some(Request::Snapshot) => Answer::Snapshot(self.platform.snapshot(uid)),
                Some(Request::Token { reason }) => {
                    let attestation =
                        platform::attestation(&registration.desktop_entry_id, registration.system);
                    Answer::Token(self.platform.token(uid, &attestation, reason))
                }
                Some(Request::Subscribe) => return self.subscribe(connection, registration),
                Some(Request::Remove {} | Request::ErasePreview {} | Request::EraseAll { .. })
                    if !is_settings(registration) =>
                {
                    Answer::refused(Reason::NotPermitted)
                }
                Some(Request::ErasePreview {}) => {
                    log(&Event::ErasePreviewed);
                    Answer::ErasePreview(self.eraser.preview(&self.platform, uid))
                }
                Some(Request::EraseAll { password }) => {
                    match self.eraser.erase_all(&self.platform, uid, password) {
                        Ok(erasing) => {
                            log(&Event::Erasing {
                                signed_out: erasing.signed_out.len(),
                                not_signed_out: erasing.not_signed_out.len(),
                            });
                            let sent = connection.send(&Answer::Erasing(erasing));
                            self.eraser.restart();
                            return sent;
                        }
                        Err(refusal) => {
                            let answer = Answer::from(refusal);
                            if let Answer::Refused { reason, .. } = &answer {
                                log(&Event::EraseRefused(*reason));
                            }
                            answer
                        }
                    }
                }
                Some(Request::Remove {}) => match self.platform.remove(uid) {
                    Ok(enrollment) => {
                        log(&Event::Removed);
                        let sent = connection.send(&Answer::Removed);
                        self.platform.finish_removal(&enrollment);
                        return sent;
                    }
                    Err(error) => {
                        eprintln!("oxyos-accountd: uid {uid}: not removed: {error:?}");
                        Answer::refused(Reason::Failed)
                    }
                },
                Some(Request::Register { .. } | Request::Check { .. }) => {
                    Answer::refused(Reason::NotPermitted)
                }
                // The lock screen reads which field to show; Settings changes it.
                Some(Request::LocalSignIn) if registration.system => {
                    match self.platform.local_sign_in(uid) {
                        Ok(Some(view)) => Answer::LocalSignIn(view),
                        _ => Answer::refused(Reason::Failed),
                    }
                }
                Some(
                    Request::SetPin { .. }
                    | Request::RemovePin { .. }
                    | Request::SetPasswordSignIn { .. },
                ) if !is_settings(registration) => Answer::refused(Reason::NotPermitted),
                Some(
                    request @ (Request::SetPin { .. }
                    | Request::RemovePin { .. }
                    | Request::SetPasswordSignIn { .. }),
                ) => self.change_local(uid, request, log),
                Some(Request::SavePassword {
                    username,
                    password,
                    domain,
                }) => self.save_password(registration, username, password, domain, log),
                // Passkeys (ADR 0050 decision 7): any registered application, for the relying
                // parties associated with it, each verified by the person on the session's sheet.
                Some(request) if keychain::is_passkey_request(&request) => {
                    match (self.platform.keychains(), self.platform.enrollment_of(uid)) {
                        (Some(keychains), Ok(Some(enrollment))) => {
                            let gone = || hung_up(connection) || !self.live(registration);
                            let asker = keychain::Asker {
                                desktop_entry_id: &registration.desktop_entry_id,
                                pid: registration.pid,
                                gone: &gone,
                            };
                            keychains.passkey(uid, enrollment.user.as_str(), &asker, request)
                        }
                        (None, _) => Answer::refused(Reason::Locked),
                        _ => Answer::refused(Reason::Failed),
                    }
                }
                // The keychain is Settings' alone to manage (ADR 0050 decision 5), and AutoFill
                // the registrar's: any other application never learns whether it holds anything.
                Some(request) if keychain::is_keychain_request(&request) => {
                    let settings = registration.system
                        && registration.desktop_entry_id == platform::OWN_APPLICATIONS[0];
                    match (
                        settings && !keychain::is_registrar_request(&request),
                        self.platform.keychains(),
                        self.platform.enrollment_of(uid),
                    ) {
                        (true, Some(keychains), Ok(Some(enrollment))) => {
                            keychains.settings(uid, enrollment.user.as_str(), request)
                        }
                        (true, None, _) => Answer::refused(Reason::Locked),
                        (true, _, _) => Answer::refused(Reason::Failed),
                        (false, _, _) => Answer::refused(Reason::NotPermitted),
                    }
                }
                Some(_) => Answer::refused(Reason::NotPermitted),
                None => Answer::refused(Reason::InvalidRequest),
            };
            connection.send(&answer)?;
        }
    }

    /// The snapshot now, and again on every change, until the client closes or its registration
    /// ends. A message it sends is read and answered `invalidRequest`, and ends it.
    fn subscribe(
        &self,
        connection: &Connection,
        registration: &Arc<Registration>,
    ) -> io::Result<()> {
        {
            let mut subscriptions = registration
                .subscriptions
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if *subscriptions >= MAX_SUBSCRIPTIONS {
                return connection.send(&Answer::refused(Reason::TooMany));
            }
            *subscriptions += 1;
        }
        let subscribed = self.subscribed(connection, registration);
        *registration
            .subscriptions
            .lock()
            .unwrap_or_else(|p| p.into_inner()) -= 1;
        subscribed
    }

    fn subscribed(
        &self,
        connection: &Connection,
        registration: &Arc<Registration>,
    ) -> io::Result<()> {
        let uid = registration.uid;
        let waker = Arc::new(eventfd()?);
        self.platform.watch(uid, &waker);
        let mut sent = self.platform.snapshot(uid);
        connection.send(&Answer::Snapshot(sent.clone()))?;
        connection.set_receive_timeout(None)?;
        loop {
            let mut fds = [
                PollFd::new(connection, PollFlags::IN),
                PollFd::new(&*waker, PollFlags::IN),
                PollFd::new(&registration.ended, PollFlags::IN),
            ];
            match poll(&mut fds, None) {
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => return Err(error.into()),
            }
            let (message, changed, ended) = (
                !fds[0].revents().is_empty(),
                !fds[1].revents().is_empty(),
                !fds[2].revents().is_empty(),
            );
            if message {
                return match connection.receive::<serde::de::IgnoredAny>()? {
                    None => Ok(()),
                    Some(_) => connection.send(&Answer::refused(Reason::InvalidRequest)),
                };
            }
            if changed {
                drain(&waker);
                let now = self.platform.snapshot(uid);
                if now != sent {
                    connection.send(&Answer::Snapshot(now.clone()))?;
                    sent = now;
                }
            }
            if ended {
                // Its process or its session is gone: nothing more is told.
                return Ok(());
            }
        }
    }
}

impl<A: OxyApi + Send + Sync + 'static, U: Users + Send + Sync + 'static> Sessions<A, U> {
    /// Settings' change to the local sign-in (`crate::pin`): one proof, the password or the
    /// current PIN, never both.
    #[cfg(feature = "local")]
    fn change_local(&self, uid: u32, request: Request, log: &mut impl FnMut(&Event)) -> Answer {
        use platform::{LocalChange, Proof};
        let proof = |password: Option<Secret>, pin: Option<Secret>| match (password, pin) {
            (Some(password), None) => Some(Proof::Password(password)),
            (None, Some(pin)) => Some(Proof::Pin(pin)),
            _ => None,
        };
        let (proof, change) = match request {
            Request::SetPin {
                pin,
                password,
                current_pin,
            } => (proof(password, current_pin), LocalChange::SetPin(pin)),
            Request::RemovePin { password } => {
                (Some(Proof::Password(password)), LocalChange::RemovePin)
            }
            Request::SetPasswordSignIn {
                enabled,
                password,
                current_pin,
            } => (
                proof(password, current_pin),
                LocalChange::PasswordSignIn(enabled),
            ),
            _ => return Answer::refused(Reason::InvalidRequest),
        };
        let Some(proof) = proof else {
            return Answer::refused(Reason::InvalidRequest);
        };
        let what = change.name();
        let answer = match self
            .platform
            .change_local(uid, proof, change, self.eraser.passwords())
        {
            Ok(view) => Answer::LocalSignIn(view),
            Err(refusal) => Answer::from(refusal),
        };
        log(&Event::LocalChanged {
            change: what,
            refused: match &answer {
                Answer::Refused { reason, .. } => Some(*reason),
                _ => None,
            },
        });
        answer
    }

    #[cfg(not(feature = "local"))]
    fn change_local(&self, _uid: u32, _request: Request, _log: &mut impl FnMut(&Event)) -> Answer {
        Answer::refused(Reason::Failed)
    }
}

/// Whether the peer of `connection` closed it (a ceremony it asked for is then abandoned).
fn hung_up(connection: &Connection) -> bool {
    let mut fds = [PollFd::new(connection, PollFlags::RDHUP)];
    matches!(poll(&mut fds, Some(&rustix::time::Timespec { tv_sec: 0, tv_nsec: 0 })), Ok(n) if n > 0)
        && fds[0]
            .revents()
            .intersects(PollFlags::RDHUP | PollFlags::HUP | PollFlags::ERR)
}

/// Whether a registration is Settings', registered as a system application: the one that may
/// remove its account and erase the machine.
fn is_settings(registration: &Registration) -> bool {
    registration.system && registration.desktop_entry_id == platform::OWN_APPLICATIONS[0]
}

/// A desktop entry id as Oxy's registry takes one: `so.oxy.dropsea`.
pub fn valid_entry_id(id: &str) -> bool {
    let mut parts = id.split('.');
    let first = parts.next().unwrap_or("");
    let rest: Vec<&str> = parts.collect();
    id.len() <= 255
        && !first.is_empty()
        && first
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && !rest.is_empty()
        && rest.iter().all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

/// Answers a peer that may do nothing here: its first message read, then refused.
fn refuse(connection: &Connection) -> io::Result<()> {
    crate::ipc::refuse(connection, crate::ipc::Reason::NotPermitted)
}

/// How long the session's clients wait for an answer to a registration.
pub const REGISTER_WAIT: Duration = Duration::from_secs(5);

/// The session's connection as a registrar: one for its life.
pub struct Registrar {
    connection: Connection,
}

impl Registrar {
    pub fn connect(socket: &Path) -> io::Result<Self> {
        let connection = Connection::connect(socket)?;
        connection.set_timeout(REGISTER_WAIT)?;
        Ok(Self { connection })
    }

    /// Registers the process `pidfd` is, launched as `desktop_entry_id`.
    pub fn register(
        &self,
        desktop_entry_id: &str,
        system: bool,
        pidfd: BorrowedFd<'_>,
    ) -> io::Result<Result<(), Reason>> {
        self.connection.send_with(
            &Request::Register {
                desktop_entry_id: desktop_entry_id.to_owned(),
                system,
            },
            pidfd,
        )?;
        match self.connection.receive::<Answer>()? {
            Some(Some(Answer::Registered)) => Ok(Ok(())),
            Some(Some(Answer::Refused { reason, .. })) => Ok(Err(reason)),
            Some(_) => Err(io::Error::new(io::ErrorKind::InvalidData, "not an answer")),
            None => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the service closed the connection",
            )),
        }
    }
}

impl Registrar {
    /// Presents the session: the service answers whether it takes this connection's process for
    /// the session's registrar.
    pub fn present(&self) -> io::Result<Result<(), Reason>> {
        self.connection.send(&Request::Present {})?;
        match self.connection.receive::<Answer>()? {
            Some(Some(Answer::Accepted)) => Ok(Ok(())),
            Some(Some(Answer::Refused { reason, .. })) => Ok(Err(reason)),
            Some(_) => Err(io::Error::new(io::ErrorKind::InvalidData, "not an answer")),
            None => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the service closed the connection",
            )),
        }
    }

    /// Locks the session's keychain (the session locked).
    pub fn keychain_lock(&self) -> io::Result<Result<(), Reason>> {
        self.connection.send(&Request::KeychainLock {})?;
        match self.connection.receive::<Answer>()? {
            Some(Some(Answer::KeychainLocked)) => Ok(Ok(())),
            Some(Some(Answer::Refused { reason, .. })) => Ok(Err(reason)),
            Some(_) => Err(io::Error::new(io::ErrorKind::InvalidData, "not an answer")),
            None => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the service closed the connection",
            )),
        }
    }
}

impl AsFd for Registrar {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.connection.as_fd()
    }
}

/// One request to the session socket and its one answer, within `wait`.
pub fn ask(socket: &Path, request: &Request, wait: Duration) -> io::Result<Answer> {
    let connection = Connection::connect(socket)?;
    connection.set_timeout(wait)?;
    connection.send(request)?;
    match connection.receive::<Answer>()? {
        Some(Some(answer)) => Ok(answer),
        Some(None) => Err(io::Error::new(io::ErrorKind::InvalidData, "not an answer")),
        None => Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the service closed the connection",
        )),
    }
}

/// The lock's online check of the session's own account, after PAM said `local`. Fails if the
/// service does not answer or refuses: the lock then keeps PAM's answer.
pub fn check(socket: &Path, password: Secret, local: Local) -> io::Result<Checked> {
    match ask(
        socket,
        &Request::Check { password, local },
        crate::ipc::CHECK_WAIT,
    )? {
        Answer::Checked(checked) => Ok(checked),
        Answer::Refused { reason, .. } => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("refused: {reason:?}"),
        )),
        _ => Err(io::Error::new(io::ErrorKind::InvalidData, "not an answer")),
    }
}

/// The session's registrar, on a thread of its own: a launch hands it the process and goes on,
/// never waiting for the service. It keeps one connection for the session's life, presents the
/// session on it (`present`), and registers on it each process it was given while the process
/// lives. If the service goes (restarted: a package upgrade does it) or does not take the session,
/// it connects again, with waits doubling from 1 s to [`RECONNECT_MAX`] ([`RECONNECT_REFUSED_MAX`]
/// while the service refuses it), and at each launch, for as long as the session runs: it never
/// gives up. Connected again, it registers every process still running. What it could not do is
/// said once per outage and cause, and its end is said too. Without the service nothing is
/// registered, and applications simply get no token meanwhile.
pub struct RegistrarThread {
    launches: std::sync::mpsc::Sender<Job>,
    wake: std::os::unix::net::UnixStream,
}

/// What the registrar's thread is given to do.
enum Job {
    Register(String, bool, OwnedFd),
    /// The session locked: its keychain locks too (ADR 0050 decision 2).
    KeychainLock,
}

/// The longest wait before connecting again to a service that is not there.
pub const RECONNECT_MAX: Duration = Duration::from_secs(30);
/// The longest wait before presenting the session again to a service that refused it (one
/// started since, that knows it, may take it).
pub const RECONNECT_REFUSED_MAX: Duration = Duration::from_secs(300);

/// The wait before the `attempt`th attempt (from 0) to connect again: 1 s, doubling, up to
/// [`RECONNECT_MAX`] (or [`RECONNECT_REFUSED_MAX`] if the service `refused` the session); never
/// none.
pub fn reconnect_wait(attempt: u32, refused: bool) -> Duration {
    let most = if refused {
        RECONNECT_REFUSED_MAX
    } else {
        RECONNECT_MAX
    };
    Duration::from_secs(1u64 << attempt.min(16)).min(most)
}

/// Why the registrar is not connected.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Outage {
    /// The service could not be reached, or its connection broke.
    Unreachable(String),
    /// The service refused the session.
    Refused(Reason),
}

impl RegistrarThread {
    /// The registrar of the service at `socket`; `log` says what it could not do.
    pub fn spawn(socket: PathBuf, log: impl Fn(&str) + Send + 'static) -> io::Result<Self> {
        let (launches, received) = std::sync::mpsc::channel();
        let (wake, woken) = std::os::unix::net::UnixStream::pair()?;
        woken.set_nonblocking(true)?;
        std::thread::Builder::new()
            .name("oxyos-registrar".into())
            .spawn(move || registrar_thread(&socket, &received, &woken, &log))?;
        Ok(Self { launches, wake })
    }

    /// Registers the process `pidfd` is, launched as `desktop_entry_id` (`system`: a system
    /// application of the image's policy).
    pub fn register(&self, desktop_entry_id: &str, system: bool, pidfd: OwnedFd) {
        self.send(Job::Register(desktop_entry_id.to_owned(), system, pidfd));
    }

    /// Tells the service the session locked, so the account's keychain locks with it. Without
    /// the service there is nothing unlocked to lock.
    pub fn keychain_lock(&self) {
        self.send(Job::KeychainLock);
    }

    fn send(&self, job: Job) {
        if self.launches.send(job).is_ok() {
            use std::io::Write;
            let _ = (&self.wake).write(&[1]);
        }
    }
}

/// A connection to the service at `socket`, the session presented on it and every process of
/// `live` registered on it again.
fn reconnect(
    socket: &Path,
    live: &[(String, bool, OwnedFd)],
    log: &dyn Fn(&str),
) -> Result<Registrar, Outage> {
    let registrar = Registrar::connect(socket)
        .map_err(|error| Outage::Unreachable(format!("{}: {error}", socket.display())))?;
    match registrar.present() {
        Ok(Ok(())) => {}
        Ok(Err(reason)) => return Err(Outage::Refused(reason)),
        Err(error) => {
            return Err(Outage::Unreachable(format!(
                "presenting the session: {error}"
            )))
        }
    }
    for (id, system, pidfd) in live {
        if !register_one(&registrar, id, *system, pidfd, log) {
            return Err(Outage::Unreachable(format!(
                "the connection ended registering {id}"
            )));
        }
    }
    Ok(registrar)
}

/// The registrar's state between connections.
struct Connecting {
    /// The next attempt, and how many were made since the connection was lost.
    retry: Option<(std::time::Instant, u32)>,
    /// Why it is not connected, as said last (None: connected, or never tried).
    said: Option<Outage>,
}

impl Connecting {
    /// An attempt failed for `outage`: said if it is news, and the next one set.
    fn failed(&mut self, outage: Outage, log: &dyn Fn(&str)) {
        let attempt = self.retry.map_or(0, |(_, made)| made);
        let wait = reconnect_wait(attempt, matches!(outage, Outage::Refused(_)));
        if self.said.as_ref() != Some(&outage) {
            match &outage {
                Outage::Unreachable(why) => log(&format!(
                    "{why}; launches are registered once it is back \
                     (trying again in {} s, and at each launch)",
                    wait.as_secs()
                )),
                Outage::Refused(reason) => log(&format!(
                    "it refused the session: {reason:?}; launches are not \
                     registered (trying again in {} s, and at each launch)",
                    wait.as_secs()
                )),
            }
            self.said = Some(outage);
        }
        self.retry = Some((std::time::Instant::now() + wait, attempt.saturating_add(1)));
    }

    /// Connected: said if it was not before.
    fn connected(&mut self, registered: usize, log: &dyn Fn(&str)) {
        if self.said.take().is_some() {
            log(&format!(
                "connected again; its {registered} running applications \
                 registered again"
            ));
        }
        self.retry = None;
    }

    /// The connection was lost: tried again after a second (a service restarting is back by
    /// then), then with the waits.
    fn lost(&mut self, log: &dyn Fn(&str)) {
        let outage = Outage::Unreachable("the service closed the session's connection".into());
        if self.said.as_ref() != Some(&outage) {
            log("it closed the session's connection: connecting again");
            self.said = Some(outage);
        }
        self.retry = Some((std::time::Instant::now() + reconnect_wait(0, false), 0));
    }
}

fn registrar_thread(
    socket: &Path,
    launches: &std::sync::mpsc::Receiver<Job>,
    woken: &std::os::unix::net::UnixStream,
    log: &dyn Fn(&str),
) {
    use std::io::Read;
    use std::time::Instant;
    let mut live: Vec<(String, bool, OwnedFd)> = Vec::new();
    let mut registrar: Option<Registrar> = None;
    let mut state = Connecting {
        retry: None,
        said: None,
    };
    loop {
        let timeout = state.retry.map(|(at, _)| {
            let wait = at.saturating_duration_since(Instant::now());
            rustix::time::Timespec {
                tv_sec: wait.as_secs() as i64,
                tv_nsec: i64::from(wait.subsec_nanos()),
            }
        });
        let mut fds = vec![PollFd::new(woken, PollFlags::IN)];
        if let Some(registrar) = &registrar {
            fds.push(PollFd::new(registrar, PollFlags::IN));
        }
        let first_pidfd = fds.len();
        fds.extend(
            live.iter()
                .map(|(_, _, pidfd)| PollFd::new(pidfd, PollFlags::IN)),
        );
        if let Err(error) = poll(&mut fds, timeout.as_ref()) {
            if error != rustix::io::Errno::INTR {
                // Never a reason to stop registering: said, and waited on again after a moment.
                log(&format!("waiting: {error}"));
                drop(fds);
                std::thread::sleep(Duration::from_secs(1));
            }
            continue;
        }
        let woke = !fds[0].revents().is_empty();
        let lost = registrar.is_some() && !fds[1].revents().is_empty();
        let exited: Vec<bool> = fds[first_pidfd..]
            .iter()
            .map(|fd| !fd.revents().is_empty())
            .collect();
        drop(fds);
        // Processes that exited are forgotten (the service saw them go too).
        let mut exited = exited.into_iter();
        live.retain(|_| !exited.next().unwrap_or(false));
        if lost {
            // The service closed the connection (it stopped): its registrations went with it.
            registrar = None;
            state.lost(log);
        }
        let mut launched = false;
        if woke {
            let mut bytes = [0u8; 64];
            // The session let go of its registrar: it is ending.
            if let Ok(0) = (&*woken).read(&mut bytes) {
                return;
            }
            while let Ok(job) = launches.try_recv() {
                let (id, system, pidfd) = match job {
                    Job::Register(id, system, pidfd) => (id, system, pidfd),
                    Job::KeychainLock => {
                        if let Some(connected) = &registrar {
                            if !lock_keychain(connected, log) {
                                registrar = None;
                                state.lost(log);
                            }
                        }
                        continue;
                    }
                };
                if let Some(connected) = &registrar {
                    if !register_one(connected, &id, system, &pidfd, log) {
                        registrar = None;
                        state.lost(log);
                    }
                }
                live.push((id, system, pidfd));
                launched = true;
            }
        }
        // Not connected: a launch is a time to try again, as is the wait's end.
        let due = state.retry.is_some_and(|(at, _)| Instant::now() >= at);
        if registrar.is_none() && (launched || due || state.retry.is_none() && woke) {
            match reconnect(socket, &live, log) {
                Ok(connected) => {
                    state.connected(live.len(), log);
                    registrar = Some(connected);
                }
                Err(outage) => state.failed(outage, log),
            }
        }
    }
}

/// Locks the session's keychain; false if the connection failed (the service went, and with it
/// whatever it had unlocked).
fn lock_keychain(registrar: &Registrar, log: &dyn Fn(&str)) -> bool {
    match registrar.keychain_lock() {
        Ok(Ok(())) => true,
        Ok(Err(reason)) => {
            log(&format!("the keychain was not locked: {reason:?}"));
            true
        }
        Err(error) => {
            log(&format!("locking the keychain: {error}"));
            false
        }
    }
}

/// Registers one process; false if the connection failed (the service went).
fn register_one(
    registrar: &Registrar,
    id: &str,
    system: bool,
    pidfd: &OwnedFd,
    log: &dyn Fn(&str),
) -> bool {
    match registrar.register(id, system, pidfd.as_fd()) {
        Ok(Ok(())) => true,
        Ok(Err(reason)) => {
            log(&format!("{id} was not registered: {reason:?}"));
            true
        }
        Err(error) => {
            log(&format!("registering {id}: {error}"));
            false
        }
    }
}
