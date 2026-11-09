//! The service's keychains ([ADR 0050](../../../docs/adr/0050-keychain-autofill-and-passkeys.mdx),
//! decisions 1 to 5, step 2): one per enrolled account, in `/var/lib/oxyos/keychain/<user>/`,
//! open only while that account's session is.
//!
//! - **Unlocking** with the account's password (the login program's `unlockKeychain`, the
//!   registrar's accepted `check`): the keybag opened (or made, the first time), the vault kept on
//!   the machine loaded, every key locked in memory ([`oxyos_keychain::memory`]; a key that cannot
//!   be is a keychain that stays locked, unless the service was told it may not lock memory: the
//!   tests'). A password that does not open the keybag (changed elsewhere) is kept, locked in
//!   memory, until Settings gives the previous one (`keychainRekey`) or leaves (`keychainLeave`).
//! - **Locking** drops the keybag key, the keyring, the member's keys, the refs and whatever was
//!   being made, at once: when the registrar says the session locked, when its session's last
//!   registrar connection closes, when the account is removed, and when the login program unlocked
//!   it and no session came within [`GRACE`].
//! - **Oxy** is called with OxyOS's own application token, never while the keychain's state is
//!   held: each operation prepares under the lock, calls without it, and applies the answer under
//!   it again only if the keychain was not locked meanwhile (its epoch). An account's calls are made
//!   one at a time.
//! - **What is logged**: the account's user, what was asked and why it failed (a reason, Oxy's
//!   status and code, the crate's kind of error); never a password, key, code, item, name or
//!   username.

use super::api::{VaultApi, VaultError, VaultResult};
use super::*;
use crate::secret::Secret;
use crate::session::{valid_entry_id, Answer, Reason, Request};
use oxyos_keychain::item::{
    Content, Item, Login, Note, PreviousPassword, Totp as TotpItem, Url, Wifi, WifiSecurity,
    PASSWORD_HISTORY,
};
use oxyos_keychain::memory::Pinned;
use oxyos_keychain::recovery::RecoveryKey;
use oxyos_keychain::server::codes;
use oxyos_keychain::store::{Store, Unlocked};
use oxyos_keychain::vault::{
    self, Approving, Created, JoinStep, Joining, Look, RecoveryJoin, Rotation, Vault,
};
use oxyos_keychain::{EnrollmentId, ItemId, MemberId};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

/// Where the keychains are kept: a directory per account's user under it.
pub const STATE_DIRECTORY: &str = oxyos_keychain::store::STATE_DIRECTORY;

mod passkeys;
pub use passkeys::{RelyingParties, WebOrigins, BROWSERS};

/// How long a keychain the login program unlocked waits for its session's registrar.
pub const GRACE: Duration = Duration::from_secs(120);
/// How long Settings' verification lasts.
pub const VERIFIED_FOR: Duration = Duration::from_secs(5 * 60);
/// A member's vault is synced again when Settings looks and it is older than this.
pub const SYNC_AFTER: Duration = Duration::from_secs(60);
/// At most this many refs at once; past it, the oldest die with the rest.
const MAX_REFS: usize = 16_384;
/// Wrong passwords given free, then a wait doubling from [`THROTTLE_FIRST`] to [`THROTTLE_MOST`].
const THROTTLE_FREE: u32 = 5;
const THROTTLE_FIRST: Duration = Duration::from_secs(30);
const THROTTLE_MOST: Duration = Duration::from_secs(15 * 60);

/// OxyOS's own token for an account's uid; `true` asks for a new one (the last was refused).
pub type TokenSource = Box<dyn Fn(u32, bool) -> Option<Secret> + Send + Sync>;

/// Whether keys must be locked in memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Memory {
    /// The service's: a key that cannot be locked keeps the keychain locked.
    Locked,
    /// The tests': locked where the process may, else not.
    BestEffort,
}

pub struct KeychainService<V> {
    api: V,
    directory: PathBuf,
    /// This machine as a member names itself.
    name: String,
    token: TokenSource,
    memory: Memory,
    users: Mutex<HashMap<u32, Arc<User>>>,
    /// Which relying parties each application may use (passkeys).
    parties: Mutex<Arc<RelyingParties>>,
    me: Weak<Self>,
}

struct User {
    name: String,
    data: Mutex<Data>,
    /// The account's calls to Oxy, one at a time.
    net: Mutex<()>,
    /// The session's passkey sheets, and the ceremony shown.
    prompts: Mutex<passkeys::Prompts>,
    prompted: Condvar,
}

struct Data {
    /// Moves with every unlock and lock: an answer from Oxy is applied only in the epoch it was
    /// asked in.
    epoch: u64,
    phase: Phase,
    /// The session's registrar connections open.
    sessions: usize,
    /// Unlocked without a session: locked at this time unless one comes.
    grace_until: Option<Instant>,
    throttle: Throttle,
}

enum Phase {
    Locked,
    /// A password that does not open the keybag, the one the machine has now (kept to re-seal or
    /// make a new keybag with).
    NeedsPrevious(Option<Learned>),
    Open(Box<Open>),
}

/// A password learned, locked in memory while held.
struct Learned {
    password: Box<Zeroizing<String>>,
    _pinned: Option<Pinned>,
}

struct Open {
    store: Unlocked,
    vault: Option<Vault>,
    /// Off, Member, Removed or Unbound (Creating and Joining are what is pending).
    standing: KeychainState,
    /// Whether Oxy has a vault for the account, once asked.
    vault_known: Option<bool>,
    refs: HashMap<String, ItemId>,
    by_item: HashMap<ItemId, String>,
    verified_until: Option<Instant>,
    creating: Option<Created>,
    joining: Option<Joining>,
    recovering: Option<RecoveryJoin>,
    approving: HashMap<EnrollmentId, Approving>,
    rotating: Option<Rotation>,
    /// The last sync, or attempt at one.
    last_sync: Option<Instant>,
}

impl Open {
    fn new(store: Unlocked, vault: Option<Vault>) -> Self {
        let standing = if vault.is_some() {
            KeychainState::Member
        } else {
            KeychainState::Off
        };
        Self {
            store,
            vault,
            standing,
            vault_known: None,
            refs: HashMap::new(),
            by_item: HashMap::new(),
            verified_until: None,
            creating: None,
            joining: None,
            recovering: None,
            approving: HashMap::new(),
            rotating: None,
            last_sync: None,
        }
    }

    fn state(&self) -> KeychainState {
        if self.creating.is_some() {
            KeychainState::Creating
        } else if self.joining.is_some() {
            KeychainState::Joining
        } else {
            self.standing
        }
    }

    fn verified(&self) -> bool {
        self.verified_until
            .is_some_and(|until| until > Instant::now())
    }

    /// Whether Settings proved the person, and if so another five minutes from now: the proof
    /// lasts until five minutes pass with nothing asked of it (ADR 0050 decision 8), or the lock.
    fn still_verified(&mut self) -> bool {
        let verified = self.verified();
        if verified {
            self.verified_until = Some(Instant::now() + VERIFIED_FOR);
        }
        verified
    }

    fn member(&mut self) -> Result<&mut Vault, Reason> {
        member_of(&mut self.vault, self.standing)
    }

    /// The vault's id is forgotten here: the machine is no member any more.
    fn forget_vault(&mut self, standing: KeychainState) {
        if let Err(error) = self.store.forget_vault() {
            eprintln!("oxyos-accountd: the keychain's vault cannot be forgotten: {error}");
        }
        self.vault = None;
        self.refs.clear();
        self.by_item.clear();
        self.approving.clear();
        self.rotating = None;
        self.standing = standing;
    }

    /// The ref of an item for this unlock.
    fn reference(&mut self, id: &ItemId) -> Result<String, Reason> {
        if let Some(reference) = self.by_item.get(id) {
            return Ok(reference.clone());
        }
        if self.refs.len() >= MAX_REFS {
            self.refs.clear();
            self.by_item.clear();
        }
        let random = oxyos_keychain::enroll::nonce().map_err(|_| Reason::Failed)?;
        let reference = oxyos_keychain::wire::b64(&random[..18]);
        self.refs.insert(reference.clone(), id.clone());
        self.by_item.insert(id.clone(), reference.clone());
        Ok(reference)
    }
}

/// The vault, if this machine is a member in good standing.
fn member_of(vault: &mut Option<Vault>, standing: KeychainState) -> Result<&mut Vault, Reason> {
    match (vault, standing) {
        (Some(vault), KeychainState::Member) => Ok(vault),
        (_, KeychainState::Removed) => Err(Reason::Removed),
        _ => Err(Reason::NotMember),
    }
}

#[derive(Default)]
struct Throttle {
    failures: u32,
    until: Option<Instant>,
}

impl Throttle {
    fn waiting(&self) -> bool {
        self.until.is_some_and(|until| until > Instant::now())
    }

    /// Seconds until another attempt may be made, while waiting.
    fn retry_after(&self) -> Option<u64> {
        let left = self.until?.checked_duration_since(Instant::now())?;
        Some(left.as_secs() + u64::from(left.subsec_nanos() > 0))
    }

    fn record(&mut self, accepted: bool) {
        if accepted {
            *self = Self::default();
            return;
        }
        self.failures += 1;
        if self.failures >= THROTTLE_FREE {
            let doublings = (self.failures - THROTTLE_FREE).min(5);
            let wait = (THROTTLE_FIRST * 2u32.pow(doublings)).min(THROTTLE_MOST);
            self.until = Some(Instant::now() + wait);
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|p| p.into_inner())
}

fn refused(reason: Reason) -> Answer {
    Answer::refused(reason)
}

fn answer(result: Result<Answer, Reason>) -> Answer {
    result.unwrap_or_else(refused)
}

/// What a failed call to Oxy is told as.
fn oxy_reason(error: &VaultError) -> Reason {
    match (error, error.code()) {
        (VaultError::Offline(_), _) => Reason::Offline,
        (_, Some(codes::EXISTS | codes::ALREADY_MEMBER)) => Reason::VaultExists,
        (_, Some(codes::NOT_FOUND | codes::NOT_MEMBER)) => Reason::NotMember,
        (_, Some(codes::ENROLLMENT_EXPIRED)) => Reason::Expired,
        (_, Some(codes::ENROLLMENT_NOT_FOUND | codes::ITEM_NOT_FOUND)) => Reason::NotFound,
        (_, Some(codes::ROSTER_CONFLICT | codes::ITEM_CONFLICT)) => Reason::Conflict,
        _ if error.status() == Some(410) => Reason::Expired,
        _ => Reason::Oxy,
    }
}

/// What a refusal of the keychain's own checks is told as.
fn keychain_reason(error: &oxyos_keychain::Error) -> Reason {
    use oxyos_keychain::Error;
    match error {
        Error::WrongPassword => Reason::WrongPassword,
        Error::NotThisRecoveryKey => Reason::RecoveryKeyMismatch,
        Error::Memory(_) | Error::Io(_) | Error::Unsafe(_) | Error::Random | Error::Exists => {
            Reason::Failed
        }
        _ => Reason::Oxy,
    }
}

fn platform_name(member: &oxyos_keychain::roster::Member) -> String {
    serde_json::to_value(member.platform)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// A URL's host, or the text itself when it is none.
fn host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_owned))
        .unwrap_or_else(|| url.to_owned())
}

impl<V: VaultApi + 'static> KeychainService<V> {
    /// The keychains of every account, kept in `directory` (`/var/lib/oxyos/keychain`), this
    /// machine named `name` as a member, OxyOS's own tokens from `token`.
    pub fn new(
        api: V,
        directory: PathBuf,
        name: String,
        token: TokenSource,
        memory: Memory,
    ) -> Arc<Self> {
        let service = Arc::new_cyclic(|me| Self {
            api,
            directory,
            name,
            token,
            memory,
            users: Mutex::new(HashMap::new()),
            parties: Mutex::new(Arc::new(RelyingParties::system())),
            me: me.clone(),
        });
        let weak = Arc::downgrade(&service);
        let _ = std::thread::Builder::new()
            .name("oxyos-accountd-keychain".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_secs(2));
                let Some(service) = weak.upgrade() else {
                    return;
                };
                service.sweep();
            });
        service
    }

    pub fn api(&self) -> &V {
        &self.api
    }

    /// Locks every keychain unlocked without a session past its grace.
    pub fn sweep(&self) {
        self.sweep_at(Instant::now());
    }

    /// [`KeychainService::sweep`] as it would be at `now`.
    pub fn sweep_at(&self, now: Instant) {
        let users: Vec<Arc<User>> = lock(&self.users).values().cloned().collect();
        for user in users {
            let mut data = lock(&user.data);
            if data.sessions == 0 && data.grace_until.is_some_and(|until| until <= now) {
                eprintln!(
                    "oxyos-accountd: {}'s keychain was unlocked and no session came: locked",
                    user.name
                );
                lock_data(&mut data);
            }
        }
    }

    fn user(&self, uid: u32, name: &str) -> Arc<User> {
        lock(&self.users)
            .entry(uid)
            .or_insert_with(|| {
                Arc::new(User {
                    name: name.to_owned(),
                    data: Mutex::new(Data {
                        epoch: 0,
                        phase: Phase::Locked,
                        sessions: 0,
                        grace_until: None,
                        throttle: Throttle::default(),
                    }),
                    net: Mutex::new(()),
                    prompts: Mutex::new(passkeys::Prompts::default()),
                    prompted: Condvar::new(),
                })
            })
            .clone()
    }

    fn store(&self, user: &User) -> Result<Store, Reason> {
        Store::open(&self.directory.join(&user.name)).map_err(|error| {
            eprintln!("oxyos-accountd: {}'s keychain: {error}", user.name);
            Reason::Failed
        })
    }

    /// Locks a key in memory, as the service's policy says.
    fn pin(&self, user: &User, pinned: oxyos_keychain::Result<()>) -> Result<(), Reason> {
        match (pinned, self.memory) {
            (Ok(()), _) | (Err(_), Memory::BestEffort) => Ok(()),
            (Err(error), Memory::Locked) => {
                eprintln!(
                    "oxyos-accountd: {}'s keychain stays locked: {error} (LimitMEMLOCK)",
                    user.name
                );
                Err(Reason::Failed)
            }
        }
    }

    fn failed(&self, user: &User, what: &str, error: oxyos_keychain::Error) -> Reason {
        eprintln!("oxyos-accountd: {}'s keychain: {what}: {error}", user.name);
        keychain_reason(&error)
    }

    /// Runs `f` on the open keychain, if it is still open in `epoch` (any, without one).
    fn with_open<T>(
        &self,
        user: &User,
        epoch: Option<u64>,
        f: impl FnOnce(&mut Open) -> Result<T, Reason>,
    ) -> Result<(u64, T), Reason> {
        let mut data = lock(&user.data);
        if epoch.is_some_and(|epoch| epoch != data.epoch) {
            return Err(Reason::Locked);
        }
        let epoch = data.epoch;
        match &mut data.phase {
            Phase::Open(open) => f(open).map(|value| (epoch, value)),
            Phase::NeedsPrevious(_) => Err(Reason::NeedsPreviousPassword),
            Phase::Locked => Err(Reason::Locked),
        }
    }

    /// One call to Oxy with OxyOS's own token, asked again with a new token if it was refused.
    fn call<T>(&self, uid: u32, f: impl Fn(&V, &Secret) -> VaultResult<T>) -> VaultResult<T> {
        let token = (self.token)(uid, false)
            .ok_or_else(|| VaultError::Offline("no token for OxyOS".into()))?;
        match f(&self.api, &token) {
            Err(error) if error.status() == Some(401) => {
                let token = (self.token)(uid, true)
                    .ok_or_else(|| VaultError::Offline("no token for OxyOS".into()))?;
                f(&self.api, &token)
            }
            other => other,
        }
    }

    fn ask<T>(
        &self,
        user: &User,
        uid: u32,
        what: &str,
        f: impl Fn(&V, &Secret) -> VaultResult<T>,
    ) -> Result<T, Reason> {
        self.call(uid, f).map_err(|error| {
            eprintln!("oxyos-accountd: {}'s keychain: {what}: {error}", user.name);
            oxy_reason(&error)
        })
    }

    /// Opens `store` with `password` (made the first time): the keychain as it is then.
    fn open_with(
        &self,
        user: &User,
        store: &Store,
        password: &str,
    ) -> Result<Box<Open>, oxyos_keychain::Error> {
        let mut unlocked = if store.has_keybag()? {
            store.unlock(password)?
        } else {
            store.create(password)?
        };
        let pinned = unlocked.pin();
        self.pin(user, pinned)
            .map_err(|_| oxyos_keychain::Error::Malformed("memory"))?;
        let mut vault = Vault::load(&unlocked)?;
        if let Some(vault) = &mut vault {
            let pinned = vault.pin();
            self.pin(user, pinned)
                .map_err(|_| oxyos_keychain::Error::Malformed("memory"))?;
        }
        Ok(Box::new(Open::new(unlocked, vault)))
    }

    /// Installs a keychain opened in `epoch`, if nothing locked it meanwhile.
    fn install(&self, user: &User, uid: u32, epoch: u64, open: Box<Open>) -> KeychainState {
        let mut data = lock(&user.data);
        if data.epoch != epoch {
            return state_of(&data);
        }
        let member = open.vault.is_some();
        data.epoch += 1;
        data.phase = Phase::Open(open);
        data.grace_until = (data.sessions == 0).then(|| Instant::now() + GRACE);
        let state = state_of(&data);
        drop(data);
        if member {
            self.sync_later(uid);
        }
        state
    }

    fn learned(&self, user: &User, password: &str) -> Option<Learned> {
        let password = Box::new(Zeroizing::new(password.to_owned()));
        let pinned = Pinned::new(password.as_bytes());
        match pinned {
            Ok(pinned) => Some(Learned {
                password,
                _pinned: Some(pinned),
            }),
            Err(_) if self.memory == Memory::BestEffort => Some(Learned {
                password,
                _pinned: None,
            }),
            Err(error) => {
                eprintln!(
                    "oxyos-accountd: {}: the new password cannot be held: {error}",
                    user.name
                );
                None
            }
        }
    }

    fn sync_later(&self, uid: u32) {
        let Some(service) = self.me.upgrade() else {
            return;
        };
        let _ = std::thread::Builder::new()
            .name("oxyos-accountd-keychain-sync".into())
            .spawn(move || {
                let user = lock(&service.users).get(&uid).cloned();
                if let Some(user) = user {
                    let _ = service.sync(&user, uid);
                }
            });
    }

    /// Syncs `uid`'s vault with Oxy now.
    pub fn sync_account(&self, uid: u32) -> Result<(), Reason> {
        let user = lock(&self.users).get(&uid).cloned().ok_or(Reason::Locked)?;
        self.sync(&user, uid)
    }

    /// Syncs the vault with Oxy, as [`KeychainService::sync_held`], taking the account's calls.
    fn sync(&self, user: &User, uid: u32) -> Result<(), Reason> {
        let _net = lock(&user.net);
        self.sync_held(user, uid)
    }

    /// Syncs a member's vault (the account's calls held): Oxy's roster checked against the head
    /// kept, a new generation's wrap read, orphaned members removed with rotation, then the items
    /// changed since the revision synced.
    fn sync_held(&self, user: &User, uid: u32) -> Result<(), Reason> {
        let (epoch, ()) = self.with_open(user, None, |open| open.member().map(drop))?;
        let state = self.ask(user, uid, "reading the vault", |api, token| {
            api.state(token)
        })?;
        let (_, looked) = self.with_open(user, Some(epoch), |open| {
            let vault = member_of(&mut open.vault, open.standing)?;
            let look = vault
                .look(&state)
                .map_err(|error| self.failed(user, "Oxy's roster", error))?;
            match look {
                Look::Gone => {
                    eprintln!(
                        "oxyos-accountd: {}'s vault was reset: forgotten here",
                        user.name
                    );
                    open.vault_known = Some(false);
                    open.forget_vault(KeychainState::Removed);
                    Ok(None)
                }
                Look::Removed => {
                    eprintln!(
                        "oxyos-accountd: {} was removed from its vault: forgotten here",
                        user.name
                    );
                    open.vault_known = Some(true);
                    open.forget_vault(KeychainState::Removed);
                    Ok(None)
                }
                Look::Unbound => {
                    eprintln!(
                        "oxyos-accountd: {}: Oxy binds this device to no member of the vault",
                        user.name
                    );
                    open.standing = KeychainState::Unbound;
                    Ok(None)
                }
                Look::Member(checked) => {
                    let orphaned = checked.orphaned.clone();
                    vault.adopt(checked);
                    vault
                        .save_roster(&open.store)
                        .map_err(|error| self.failed(user, "keeping the roster", error))?;
                    open.vault_known = Some(true);
                    Ok(Some((orphaned, vault.needs_wrap())))
                }
            }
        })?;
        let Some((orphaned, needs_wrap)) = looked else {
            return Ok(());
        };
        if needs_wrap {
            let wrap = self.ask(user, uid, "reading its wrap", |api, token| {
                api.own_wrap(token)
            })?;
            self.with_open(user, Some(epoch), |open| {
                let vault = member_of(&mut open.vault, open.standing)?;
                vault
                    .adopt_wrap(&wrap)
                    .map_err(|error| self.failed(user, "its wrap", error))?;
                vault
                    .save_roster(&open.store)
                    .map_err(|error| self.failed(user, "keeping the keyring", error))
            })?;
        }
        for member in orphaned {
            match self.remove_held(user, uid, epoch, &member) {
                Ok(()) => eprintln!(
                    "oxyos-accountd: {}: a member whose device session ended was removed",
                    user.name
                ),
                Err(reason) => eprintln!(
                    "oxyos-accountd: {}: an orphaned member was not removed: {reason:?}",
                    user.name
                ),
            }
        }
        loop {
            let (_, since) =
                self.with_open(user, Some(epoch), |open| Ok(open.member()?.revision()))?;
            let page = self.ask(user, uid, "reading items", |api, token| {
                api.items(token, since)
            })?;
            let (_, applied) = self.with_open(user, Some(epoch), |open| {
                let vault = member_of(&mut open.vault, open.standing)?;
                let applied = vault
                    .apply_items(since, &page)
                    .map_err(|error| self.failed(user, "Oxy's items", error))?;
                vault
                    .save_items(&open.store)
                    .map_err(|error| self.failed(user, "keeping the items", error))?;
                Ok(applied)
            })?;
            if applied.refused > 0 {
                eprintln!(
                    "oxyos-accountd: {}'s keychain: {} items from Oxy refused (a counter that \
                     went back, or a ciphertext that does not open); the copies kept stay",
                    user.name, applied.refused
                );
            }
            if applied.next.is_none() {
                break;
            }
        }
        self.with_open(user, Some(epoch), |open| {
            open.last_sync = Some(Instant::now());
            Ok(())
        })?;
        Ok(())
    }

    /// Removes `member` with the vault key's rotation (the account's calls held).
    fn remove_held(
        &self,
        user: &User,
        uid: u32,
        epoch: u64,
        member: &MemberId,
    ) -> Result<(), Reason> {
        let (_, (entries, wraps)) = self.with_open(user, Some(epoch), |open| {
            let vault = member_of(&mut open.vault, open.standing)?;
            let rotation = vault
                .removal(member, &oxyos_keychain::wire::now())
                .map_err(|error| self.failed(user, "the rotation", error))?;
            let sent = (rotation.entries.clone(), rotation.wraps.clone());
            open.rotating = Some(rotation);
            Ok(sent)
        })?;
        let state = self.ask(user, uid, "removing a member", |api, token| {
            api.append(token, &entries, &wraps)
        });
        self.with_open(user, Some(epoch), |open| {
            let rotation = open.rotating.take().ok_or(Reason::Failed)?;
            let state = state?;
            let vault = member_of(&mut open.vault, open.standing)?;
            vault
                .rotated(rotation, &state)
                .map_err(|error| self.failed(user, "Oxy's answer to the rotation", error))?;
            vault
                .save_roster(&open.store)
                .map_err(|error| self.failed(user, "keeping the rotation", error))
        })
        .map(drop)
    }

    // The registrar's.

    fn suggest(
        &self,
        open: &mut Open,
        desktop_entry_id: &str,
        field: Field,
    ) -> Result<Answer, Reason> {
        let Some(vault) = open.vault.as_ref() else {
            return Ok(Answer::AutofillSuggestions(Vec::new()));
        };
        if field == Field::NewPassword {
            return Ok(Answer::AutofillSuggestions(Vec::new()));
        }
        let mut offered = Vec::new();
        for (id, item) in vault.open_items() {
            let Ok(item) = item else { continue };
            if !vault::associated(&item, desktop_entry_id) {
                continue;
            }
            let Content::Login(login) = &item.content else {
                continue;
            };
            if field == Field::OneTimeCode && login.totp.is_none() {
                continue;
            }
            offered.push((id.clone(), login.title.clone(), login.username.clone()));
        }
        offered.sort_by(|a, b| a.1.expose().cmp(b.1.expose()));
        let mut suggestions = Vec::new();
        for (id, title, username) in offered {
            suggestions.push(Suggestion {
                reference: open.reference(&id)?,
                title,
                username,
            });
        }
        Ok(Answer::AutofillSuggestions(suggestions))
    }

    fn fill(
        open: &Open,
        desktop_entry_id: &str,
        reference: &str,
        field: Field,
    ) -> Result<Answer, Reason> {
        let id = open.refs.get(reference).ok_or(Reason::NotFound)?;
        let vault = open.vault.as_ref().ok_or(Reason::NotFound)?;
        let item = vault.open_item(id).ok().flatten().ok_or(Reason::NotFound)?;
        // Checked again: the item may have changed since it was suggested.
        if !vault::associated(&item, desktop_entry_id) {
            return Err(Reason::NotFound);
        }
        let Content::Login(login) = &item.content else {
            return Err(Reason::NotFound);
        };
        if field == Field::OneTimeCode {
            let uri = login.totp.as_ref().ok_or(Reason::NotFound)?;
            let totp =
                oxyos_keychain::totp::Totp::parse(uri.expose()).map_err(|_| Reason::NotFound)?;
            return Ok(Answer::AutofillCode {
                code: Secret::new(totp.code_now().as_str()),
            });
        }
        Ok(Answer::AutofillCredential {
            username: login.username.clone(),
            password: login.password.clone(),
        })
    }

    // Settings'.

    fn status(&self, user: &User, uid: u32) -> Answer {
        let looked = {
            let mut data = lock(&user.data);
            let epoch = data.epoch;
            match &mut data.phase {
                Phase::Open(open) => {
                    let ask = open.vault.is_none()
                        && open.state() == KeychainState::Off
                        && open.vault_known.is_none();
                    let sync = open.state() == KeychainState::Member
                        && open.last_sync.is_none_or(|at| at.elapsed() > SYNC_AFTER);
                    // The attempt counts: a sync failing (offline) is not started again by every
                    // look until a minute has passed.
                    if sync {
                        open.last_sync = Some(Instant::now());
                    }
                    Some((epoch, ask, sync))
                }
                _ => None,
            }
        };
        if let Some((epoch, ask, sync)) = looked {
            if ask {
                let _net = lock(&user.net);
                if let Ok(state) = self.call(uid, |api, token| api.state(token)) {
                    let _ = self.with_open(user, Some(epoch), |open| {
                        open.vault_known = Some(state.vault.is_some());
                        Ok(())
                    });
                }
            }
            if sync {
                self.sync_later(uid);
            }
        }
        let data = lock(&user.data);
        let (state, vault, verified, code, items) = match &data.phase {
            Phase::Locked => (KeychainState::Locked, None, false, None, None),
            Phase::NeedsPrevious(_) => (
                KeychainState::NeedsPreviousPassword,
                None,
                false,
                None,
                None,
            ),
            Phase::Open(open) => (
                open.state(),
                if open.vault.is_some() {
                    Some(true)
                } else {
                    open.vault_known
                },
                open.verified(),
                open.joining
                    .as_ref()
                    .and_then(|joining| joining.code())
                    .map(Secret::new),
                open.vault.as_ref().map(|vault| vault.open_items().count()),
            ),
        };
        Answer::KeychainStatus(Status {
            state,
            vault,
            verified,
            code,
            items,
        })
    }

    fn create(&self, user: &User, uid: u32) -> Result<Answer, Reason> {
        let _net = lock(&user.net);
        let (epoch, ()) = self.with_open(user, None, |open| match open.vault {
            Some(_) => Err(Reason::VaultExists),
            None => Ok(()),
        })?;
        let state = self.ask(user, uid, "reading the vault", |api, token| {
            api.state(token)
        })?;
        if state.vault.is_some() {
            let _ = self.with_open(user, Some(epoch), |open| {
                open.vault_known = Some(true);
                Ok(())
            });
            return Err(Reason::VaultExists);
        }
        self.with_open(user, Some(epoch), |open| {
            let mut created = vault::create(&self.name, &oxyos_keychain::wire::now())
                .map_err(|error| self.failed(user, "making the vault", error))?;
            let pinned = created.pin();
            self.pin(user, pinned)?;
            let text = created.recovery.to_text();
            open.creating = Some(created);
            open.joining = None;
            Ok(Answer::KeychainRecoveryKey {
                recovery_key: Secret::new(text.as_str()),
            })
        })
        .map(|(_, answer)| answer)
    }

    fn confirm_recovery_key(
        &self,
        user: &User,
        uid: u32,
        typed: &Secret,
    ) -> Result<Answer, Reason> {
        let _net = lock(&user.net);
        let (epoch, (genesis, wraps)) = self.with_open(user, None, |open| {
            let created = open.creating.as_ref().ok_or(Reason::InvalidRequest)?;
            let typed =
                RecoveryKey::parse(typed.expose()).map_err(|_| Reason::RecoveryKeyMismatch)?;
            if !created.recovery.matches(&typed) {
                return Err(Reason::RecoveryKeyMismatch);
            }
            Ok((created.genesis.clone(), created.wraps.clone()))
        })?;
        let state = self.call(uid, |api, token| api.create(token, &genesis, &wraps));
        self.with_open(user, Some(epoch), |open| {
            let state = match state {
                Ok(state) => state,
                Err(error) => {
                    eprintln!(
                        "oxyos-accountd: {}'s keychain: making the vault: {error}",
                        user.name
                    );
                    let reason = oxy_reason(&error);
                    if reason == Reason::VaultExists {
                        open.creating = None;
                        open.vault_known = Some(true);
                    }
                    return Err(reason);
                }
            };
            let created = open.creating.take().ok_or(Reason::Failed)?;
            let vault = created
                .accepted(&state)
                .map_err(|error| self.failed(user, "Oxy's answer to the vault made", error))?;
            vault
                .save(&open.store)
                .map_err(|error| self.failed(user, "keeping the vault", error))?;
            open.vault = Some(vault);
            open.standing = KeychainState::Member;
            open.vault_known = Some(true);
            open.last_sync = Some(Instant::now());
            Ok(())
        })?;
        eprintln!("oxyos-accountd: {} made its account's vault", user.name);
        Ok(self.status(user, uid))
    }

    fn join(
        &self,
        user: &User,
        uid: u32,
        method: JoinMethod,
        key: Option<&Secret>,
    ) -> Result<Answer, Reason> {
        let _net = lock(&user.net);
        let (epoch, pending) = self.with_open(user, None, |open| {
            if open.vault.is_some() {
                return Err(Reason::VaultExists);
            }
            Ok(open
                .joining
                .as_ref()
                .and_then(|joining| joining.enrollment().cloned()))
        })?;
        match (method, pending) {
            (JoinMethod::RecoveryKey, _) => {
                let key = key.ok_or(Reason::InvalidRequest)?;
                self.join_by_recovery(user, uid, epoch, key)
            }
            (JoinMethod::Approval, Some(id)) => self.join_progress(user, uid, epoch, &id),
            (JoinMethod::Approval, None) => {
                let state = self.ask(user, uid, "reading the vault", |api, token| {
                    api.state(token)
                })?;
                if state.vault.is_none() {
                    return Err(Reason::NotMember);
                }
                let (_, (member, commitment)) = self.with_open(user, Some(epoch), |open| {
                    let mut joining = vault::join(&state, &self.name)
                        .map_err(|error| self.failed(user, "Oxy's roster", error))?;
                    let pinned = joining.pin();
                    self.pin(user, pinned)?;
                    let sent = (joining.member().clone(), *joining.commitment());
                    open.creating = None;
                    open.joining = Some(joining);
                    Ok(sent)
                })?;
                let enrollment = self.call(uid, |api, token| {
                    api.create_enrollment(token, &member, &commitment)
                });
                self.with_open(user, Some(epoch), |open| {
                    let enrollment = enrollment.map_err(|error| {
                        eprintln!(
                            "oxyos-accountd: {}'s keychain: asking to join: {error}",
                            user.name
                        );
                        open.joining = None;
                        oxy_reason(&error)
                    })?;
                    let joining = open.joining.as_mut().ok_or(Reason::Failed)?;
                    if let Err(error) = joining.requested(&enrollment) {
                        open.joining = None;
                        return Err(self.failed(user, "Oxy's enrollment", error));
                    }
                    Ok(Answer::KeychainEnrollment(EnrollmentView {
                        enrollment_id: enrollment.id.to_string(),
                        status: EnrollmentProgress::Waiting,
                        code: None,
                        name: None,
                        platform: None,
                        expires_at: Some(enrollment.expires_at.clone()),
                    }))
                })
                .map(|(_, answer)| answer)
            }
        }
    }

    fn join_progress(
        &self,
        user: &User,
        uid: u32,
        epoch: u64,
        id: &EnrollmentId,
    ) -> Result<Answer, Reason> {
        let view = |status, code: Option<&str>| {
            Answer::KeychainEnrollment(EnrollmentView {
                enrollment_id: id.to_string(),
                status,
                code: code.map(Secret::new),
                name: None,
                platform: None,
                expires_at: None,
            })
        };
        let enrollment = self.call(uid, |api, token| api.enrollment(token, id));
        let enrollment = match enrollment {
            Ok(enrollment) => enrollment,
            Err(error) => {
                let reason = oxy_reason(&error);
                if matches!(reason, Reason::Expired | Reason::NotFound) {
                    let _ = self.with_open(user, Some(epoch), |open| {
                        open.joining = None;
                        Ok(())
                    });
                }
                eprintln!(
                    "oxyos-accountd: {}'s keychain: the enrollment: {error}",
                    user.name
                );
                return Err(reason);
            }
        };
        let progress =
            |open: &mut Open, enrollment| -> Result<(JoinStep, Option<String>, [u8; 32]), Reason> {
                let joining = open.joining.as_mut().ok_or(Reason::NotFound)?;
                match joining.progress(enrollment) {
                    Ok(JoinStep::Denied) => {
                        open.joining = None;
                        Err(Reason::Denied)
                    }
                    Ok(step) => Ok((
                        step,
                        joining.code().map(str::to_owned),
                        *joining.device_nonce(),
                    )),
                    Err(error) => {
                        open.joining = None;
                        Err(self.failed(user, "Oxy's enrollment", error))
                    }
                }
            };
        let (_, (step, code, nonce)) =
            self.with_open(user, Some(epoch), |open| progress(open, &enrollment))?;
        match step {
            JoinStep::Waiting => Ok(view(EnrollmentProgress::Waiting, None)),
            JoinStep::Code => Ok(view(EnrollmentProgress::Code, code.as_deref())),
            JoinStep::Reveal => {
                let revealed = self.call(uid, |api, token| api.reveal(token, id, &nonce));
                let revealed = match revealed {
                    Ok(revealed) => revealed,
                    Err(error) => {
                        eprintln!(
                            "oxyos-accountd: {}'s keychain: revealing: {error}",
                            user.name
                        );
                        let _ = self.with_open(user, Some(epoch), |open| {
                            open.joining = None;
                            Ok(())
                        });
                        return Err(oxy_reason(&error));
                    }
                };
                let (_, (step, code, _)) =
                    self.with_open(user, Some(epoch), |open| progress(open, &revealed))?;
                Ok(match step {
                    JoinStep::Code => view(EnrollmentProgress::Code, code.as_deref()),
                    _ => view(EnrollmentProgress::Waiting, None),
                })
            }
            JoinStep::Approved => {
                let state = self.ask(user, uid, "reading the vault", |api, token| {
                    api.state(token)
                })?;
                let wrap = self.ask(user, uid, "reading its wrap", |api, token| {
                    api.own_wrap(token)
                })?;
                self.with_open(user, Some(epoch), |open| {
                    let joining = open.joining.take().ok_or(Reason::Failed)?;
                    let mut vault = joining
                        .finish(&state, &wrap)
                        .map_err(|error| self.failed(user, "joining the vault", error))?;
                    let pinned = vault.pin();
                    self.pin(user, pinned)?;
                    vault
                        .save(&open.store)
                        .map_err(|error| self.failed(user, "keeping the vault", error))?;
                    open.vault = Some(vault);
                    open.standing = KeychainState::Member;
                    open.vault_known = Some(true);
                    Ok(())
                })?;
                eprintln!("oxyos-accountd: {} joined its account's vault", user.name);
                if let Err(reason) = self.sync_held(user, uid) {
                    eprintln!("oxyos-accountd: {}: the first sync: {reason:?}", user.name);
                }
                Ok(view(EnrollmentProgress::Approved, None))
            }
            JoinStep::Denied => Err(Reason::Denied),
        }
    }

    fn join_by_recovery(
        &self,
        user: &User,
        uid: u32,
        epoch: u64,
        key: &Secret,
    ) -> Result<Answer, Reason> {
        let mut recovery =
            RecoveryKey::parse(key.expose()).map_err(|_| Reason::RecoveryKeyMismatch)?;
        let pinned = recovery.pin();
        self.pin(user, pinned)?;
        let state = self.ask(user, uid, "reading the vault", |api, token| {
            api.state(token)
        })?;
        if state.vault.is_none() {
            return Err(Reason::NotMember);
        }
        let wrap = self.ask(user, uid, "reading the recovery wrap", |api, token| {
            api.recovery_wrap(token)
        })?;
        let (_, (entry, own)) = self.with_open(user, Some(epoch), |open| {
            let mut joining = vault::join_by_recovery(
                &state,
                &recovery,
                &wrap,
                &self.name,
                &oxyos_keychain::wire::now(),
            )
            .map_err(|error| self.failed(user, "joining with the recovery key", error))?;
            let pinned = joining.pin();
            self.pin(user, pinned)?;
            let sent = (joining.entry.clone(), joining.wrap.clone());
            open.creating = None;
            open.joining = None;
            open.recovering = Some(joining);
            Ok(sent)
        })?;
        drop(recovery);
        let state = self.call(uid, |api, token| {
            api.append(
                token,
                std::slice::from_ref(&entry),
                std::slice::from_ref(&own),
            )
        });
        self.with_open(user, Some(epoch), |open| {
            let joining = open.recovering.take().ok_or(Reason::Failed)?;
            let state = state.map_err(|error| {
                eprintln!("oxyos-accountd: {}'s keychain: joining: {error}", user.name);
                oxy_reason(&error)
            })?;
            let vault = joining
                .accepted(&state)
                .map_err(|error| self.failed(user, "Oxy's answer to joining", error))?;
            vault
                .save(&open.store)
                .map_err(|error| self.failed(user, "keeping the vault", error))?;
            open.vault = Some(vault);
            open.standing = KeychainState::Member;
            open.vault_known = Some(true);
            Ok(())
        })?;
        eprintln!(
            "oxyos-accountd: {} joined its vault with the recovery key",
            user.name
        );
        if let Err(reason) = self.sync_held(user, uid) {
            eprintln!("oxyos-accountd: {}: the first sync: {reason:?}", user.name);
        }
        Ok(self.status(user, uid))
    }

    fn enrollments(&self, user: &User, uid: u32) -> Result<Answer, Reason> {
        let _net = lock(&user.net);
        let (epoch, ()) = self.with_open(user, None, |open| open.member().map(drop))?;
        let enrollments = self.ask(user, uid, "the enrollments", |api, token| {
            api.enrollments(token)
        })?;
        self.with_open(user, Some(epoch), |open| {
            let id = open.member()?.id().clone();
            Ok(Answer::KeychainEnrollments(
                enrollments
                    .iter()
                    .filter(|enrollment| enrollment.vault_id == id)
                    .map(|enrollment| {
                        let code = open
                            .approving
                            .get(&enrollment.id)
                            .and_then(Approving::code)
                            .map(Secret::new);
                        EnrollmentView {
                            enrollment_id: enrollment.id.to_string(),
                            status: if code.is_some() {
                                EnrollmentProgress::Code
                            } else {
                                EnrollmentProgress::Waiting
                            },
                            code,
                            name: Some(enrollment.member.name.clone()),
                            platform: Some(platform_name(&enrollment.member)),
                            expires_at: Some(enrollment.expires_at.clone()),
                        }
                    })
                    .collect(),
            ))
        })
        .map(|(_, answer)| answer)
    }

    fn approve(
        &self,
        user: &User,
        uid: u32,
        id: &EnrollmentId,
        confirm: bool,
    ) -> Result<Answer, Reason> {
        let _net = lock(&user.net);
        let (epoch, (verified, started)) = self.with_open(user, None, |open| {
            open.member()?;
            Ok((open.still_verified(), open.approving.contains_key(id)))
        })?;
        if confirm {
            if !verified {
                return Err(Reason::VerificationRequired);
            }
            let (_, (entry, wrap)) = self.with_open(user, Some(epoch), |open| {
                let approving = open.approving.get(id).ok_or(Reason::NotFound)?;
                let vault = open.vault.as_ref().ok_or(Reason::NotMember)?;
                vault
                    .approval(approving, &oxyos_keychain::wire::now())
                    .map_err(|error| self.failed(user, "the approval", error))
            })?;
            let state = self.ask(user, uid, "approving", |api, token| {
                api.approve(token, id, &entry, &wrap)
            })?;
            self.with_open(user, Some(epoch), |open| {
                let approving = open.approving.remove(id).ok_or(Reason::Failed)?;
                let vault = member_of(&mut open.vault, open.standing)?;
                vault
                    .approved(&approving, &state)
                    .map_err(|error| self.failed(user, "Oxy's answer to the approval", error))?;
                vault
                    .save_roster(&open.store)
                    .map_err(|error| self.failed(user, "keeping the roster", error))
            })?;
            eprintln!(
                "oxyos-accountd: {} approved a device into its vault",
                user.name
            );
            return Ok(Answer::KeychainApproved);
        }
        let mut enrollment = self.ask(user, uid, "the enrollment", |api, token| {
            api.enrollment(token, id)
        })?;
        if !started {
            let (_, (member, nonce)) = self.with_open(user, Some(epoch), |open| {
                let vault = member_of(&mut open.vault, open.standing)?;
                let approving = vault
                    .approving(&enrollment)
                    .map_err(|error| self.failed(user, "the enrollment", error))?;
                let sent = (vault.me().clone(), *approving.nonce());
                open.approving.insert(id.clone(), approving);
                Ok(sent)
            })?;
            enrollment = match self.call(uid, |api, token| {
                api.approver_nonce(token, id, &member, &nonce)
            }) {
                Ok(enrollment) => enrollment,
                Err(error) => {
                    eprintln!(
                        "oxyos-accountd: {}'s keychain: approving: {error}",
                        user.name
                    );
                    let _ = self.with_open(user, Some(epoch), |open| {
                        open.approving.remove(id);
                        Ok(())
                    });
                    return Err(oxy_reason(&error));
                }
            };
        }
        self.with_open(user, Some(epoch), |open| {
            let vault = open.vault.as_ref().ok_or(Reason::NotMember)?;
            let approving = open.approving.get_mut(id).ok_or(Reason::NotFound)?;
            let code = match vault.approval_code(approving, &enrollment) {
                Ok(code) => code,
                Err(error) => {
                    open.approving.remove(id);
                    return Err(self.failed(user, "the enrollment", error));
                }
            };
            Ok(Answer::KeychainEnrollment(EnrollmentView {
                enrollment_id: id.to_string(),
                status: if code.is_some() {
                    EnrollmentProgress::Code
                } else {
                    EnrollmentProgress::Waiting
                },
                code: code.as_ref().map(|code| Secret::new(code.as_str())),
                name: Some(enrollment.member.name.clone()),
                platform: Some(platform_name(&enrollment.member)),
                expires_at: Some(enrollment.expires_at.clone()),
            }))
        })
        .map(|(_, answer)| answer)
    }

    fn deny(&self, user: &User, uid: u32, id: &EnrollmentId) -> Result<Answer, Reason> {
        let _net = lock(&user.net);
        let (epoch, ()) = self.with_open(user, None, |open| open.member().map(drop))?;
        self.ask(user, uid, "denying", |api, token| {
            api.delete_enrollment(token, id)
        })?;
        self.with_open(user, Some(epoch), |open| {
            open.approving.remove(id);
            Ok(Answer::KeychainDenied)
        })
        .map(|(_, answer)| answer)
    }

    fn members(&self, user: &User, uid: u32) -> Result<Answer, Reason> {
        let _net = lock(&user.net);
        let (epoch, ()) = self.with_open(user, None, |open| open.member().map(drop))?;
        let state = self.call(uid, |api, token| api.state(token));
        self.with_open(user, Some(epoch), |open| {
            let mut orphaned = Vec::new();
            if let Ok(state) = &state {
                let vault = member_of(&mut open.vault, open.standing)?;
                if let Look::Member(checked) = vault
                    .look(state)
                    .map_err(|error| self.failed(user, "Oxy's roster", error))?
                {
                    orphaned = checked.orphaned.clone();
                    vault.adopt(checked);
                    vault
                        .save_roster(&open.store)
                        .map_err(|error| self.failed(user, "keeping the roster", error))?;
                }
            }
            let vault = member_of(&mut open.vault, open.standing)?;
            Ok(Answer::KeychainMembers(
                vault
                    .roster()
                    .members()
                    .map(|member| MemberView {
                        member_id: member.id.to_string(),
                        name: member.name.clone(),
                        platform: platform_name(member),
                        own: member.id == *vault.me(),
                        orphaned: orphaned.contains(&member.id),
                    })
                    .collect(),
            ))
        })
        .map(|(_, answer)| answer)
    }

    fn remove_member(&self, user: &User, uid: u32, member: &MemberId) -> Result<Answer, Reason> {
        let _net = lock(&user.net);
        let (epoch, ()) = self.with_open(user, None, |open| {
            open.member()?;
            if !open.still_verified() {
                return Err(Reason::VerificationRequired);
            }
            let vault = open.vault.as_ref().ok_or(Reason::NotMember)?;
            if *member == *vault.me() || vault.roster().member(member).is_none() {
                return Err(Reason::NotFound);
            }
            Ok(())
        })?;
        self.remove_held(user, uid, epoch, member)?;
        eprintln!(
            "oxyos-accountd: {} removed a device from its vault",
            user.name
        );
        Ok(Answer::KeychainMemberRemoved)
    }

    /// Settings' verification: the keybag opened with `password` (Argon2id, as the unlock).
    fn verify(&self, user: &User, password: &Secret) -> Result<Answer, Reason> {
        let epoch = self.check_password(user, password)?;
        let mut data = lock(&user.data);
        if data.epoch != epoch {
            return Err(Reason::Locked);
        }
        match &mut data.phase {
            Phase::Open(open) => {
                open.verified_until = Some(Instant::now() + VERIFIED_FOR);
                Ok(Answer::KeychainVerified)
            }
            _ => Err(Reason::Locked),
        }
    }

    fn verified<T>(
        &self,
        user: &User,
        f: impl FnOnce(&mut Open) -> Result<T, Reason>,
    ) -> Result<(u64, T), Reason> {
        self.with_open(user, None, |open| {
            if !open.still_verified() {
                return Err(Reason::VerificationRequired);
            }
            f(open)
        })
    }

    fn list(
        &self,
        user: &User,
        query: Option<&Secret>,
        kind: Option<ItemKind>,
    ) -> Result<Answer, Reason> {
        let query = query.map(|query| Zeroizing::new(query.expose().to_lowercase()));
        self.verified(user, |open| {
            let Some(vault) = open.vault.as_ref() else {
                return Ok(Answer::KeychainItems(Vec::new()));
            };
            let mut items = Vec::new();
            for (id, item) in vault.open_items() {
                let Ok(item) = item else { continue };
                let summary = summary(id, &item);
                if kind.is_some_and(|kind| kind != summary.kind) {
                    continue;
                }
                if let Some(query) = &query {
                    let matches =
                        |text: &Secret| text.expose().to_lowercase().contains(query.as_str());
                    if !(matches(&summary.title)
                        || summary.username.as_ref().is_some_and(matches)
                        || summary.domains.iter().any(matches))
                    {
                        continue;
                    }
                }
                items.push(summary);
            }
            items.sort_by(|a, b| {
                a.title
                    .expose()
                    .to_lowercase()
                    .cmp(&b.title.expose().to_lowercase())
            });
            Ok(Answer::KeychainItems(items))
        })
        .map(|(_, answer)| answer)
    }

    fn reveal(&self, user: &User, id: &ItemId) -> Result<Answer, Reason> {
        self.verified(user, |open| {
            let vault = open.vault.as_ref().ok_or(Reason::NotFound)?;
            let item = vault
                .open_item(id)
                .map_err(|error| self.failed(user, "an item", error))?
                .ok_or(Reason::NotFound)?;
            Ok(Answer::KeychainItem(view(&item)))
        })
        .map(|(_, answer)| answer)
    }

    /// An item's verification code now: a login's, or a code's own. Settings asks again as each
    /// code runs out, on its own, so asking is not the person's activity: the proof is not renewed.
    fn code(&self, user: &User, id: &ItemId) -> Result<Answer, Reason> {
        self.with_open(user, None, |open| {
            if !open.verified() {
                return Err(Reason::VerificationRequired);
            }
            let vault = open.vault.as_ref().ok_or(Reason::NotFound)?;
            let item = vault
                .open_item(id)
                .map_err(|error| self.failed(user, "an item", error))?
                .ok_or(Reason::NotFound)?;
            let uri = match &item.content {
                Content::Login(login) => login.totp.as_ref().ok_or(Reason::NotFound)?,
                Content::Totp(totp) => &totp.uri,
                _ => return Err(Reason::NotFound),
            };
            let totp =
                oxyos_keychain::totp::Totp::parse(uri.expose()).map_err(|_| Reason::NotFound)?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| Reason::Failed)?
                .as_secs();
            Ok(Answer::KeychainCode(CodeView {
                code: Secret::new(totp.code_at(now).as_str()),
                period: totp.period(),
                remaining: totp.remaining(now),
            }))
        })
        .map(|(_, answer)| answer)
    }

    /// A strong password for Settings' new or edited item, the generator AutoFill offers.
    fn generate(&self, user: &User) -> Result<Answer, Reason> {
        self.verified(user, |_| {
            let password = oxyos_keychain::password::generate().map_err(|_| Reason::Failed)?;
            Ok(Answer::GeneratedPassword(password.clone()))
        })
        .map(|(_, answer)| answer)
    }

    fn put(&self, user: &User, uid: u32, input: &ItemInput) -> Result<Answer, Reason> {
        let _net = lock(&user.net);
        match self.put_held(user, uid, input) {
            Err(Reason::Conflict) => {
                // Another device wrote first: what it wrote is read, then this write is made
                // again over it.
                self.sync_held(user, uid)?;
                self.put_held(user, uid, input)
            }
            other => other,
        }
    }

    fn put_held(&self, user: &User, uid: u32, input: &ItemInput) -> Result<Answer, Reason> {
        let written = self.write_held(user, uid, &|open| {
            if !open.verified() {
                return Err(Reason::VerificationRequired);
            }
            let vault = member_of(&mut open.vault, open.standing)?;
            build(vault, input).map(Some)
        })?;
        let item_id = written.ok_or(Reason::Failed)?;
        Ok(Answer::KeychainSaved { item_id })
    }

    /// Writes the item `make` makes of the open keychain (none: nothing to write), sealed, to Oxy,
    /// then kept: its id once written. The account's calls must be held (`net`).
    fn write_held(
        &self,
        user: &User,
        uid: u32,
        make: &dyn Fn(&mut Open) -> Result<Option<Item>, Reason>,
    ) -> Result<Option<String>, Reason> {
        let (epoch, prepared) = self.with_open(user, None, |open| {
            let Some(item) = make(open)? else {
                return Ok(None);
            };
            let vault = member_of(&mut open.vault, open.standing)?;
            vault
                .prepare(&item)
                .map(Some)
                .map_err(|error| self.failed(user, "sealing an item", error))
        })?;
        let Some(prepared) = prepared else {
            return Ok(None);
        };
        let (id, base, sealed) = (prepared.id.clone(), prepared.base, prepared.sealed.clone());
        let answer = self.ask(user, uid, "writing an item", |api, token| {
            api.put_item(token, &id, base, &sealed)
        })?;
        self.with_open(user, Some(epoch), |open| {
            let vault = member_of(&mut open.vault, open.standing)?;
            vault
                .written(prepared, &answer)
                .map_err(|error| self.failed(user, "Oxy's answer to a write", error))?;
            vault
                .save_items(&open.store)
                .map_err(|error| self.failed(user, "keeping the items", error))?;
            Ok(Some(id.to_string()))
        })
        .map(|(_, written)| written)
    }

    // An application's save, once the person said yes (decision 5).

    fn save_login(&self, user: &User, uid: u32, login: &SaveLogin) -> Result<(), Reason> {
        let _net = lock(&user.net);
        let make = |open: &mut Open| saved_item(open, login);
        match self.write_held(user, uid, &make) {
            Err(Reason::Conflict) => {
                // Another device wrote first: what it wrote is read, then the save is made again
                // over it.
                self.sync_held(user, uid)?;
                self.write_held(user, uid, &make)
            }
            other => other,
        }
        .map(|_| ())
    }

    fn delete(&self, user: &User, uid: u32, id: &ItemId) -> Result<Answer, Reason> {
        let _net = lock(&user.net);
        let (epoch, base) = self.verified(user, |open| {
            let vault = member_of(&mut open.vault, open.standing)?;
            match (vault.base(id), vault.open_item(id)) {
                (Some(base), Ok(Some(_))) => Ok(base),
                _ => Err(Reason::NotFound),
            }
        })?;
        let answer = self.ask(user, uid, "deleting an item", |api, token| {
            api.delete_item(token, id, base)
        })?;
        self.with_open(user, Some(epoch), |open| {
            let vault = member_of(&mut open.vault, open.standing)?;
            vault
                .deleted(id, base, &answer)
                .map_err(|error| self.failed(user, "Oxy's answer to a deletion", error))?;
            vault
                .save_items(&open.store)
                .map_err(|error| self.failed(user, "keeping the items", error))?;
            open.by_item
                .remove(id)
                .map(|reference| open.refs.remove(&reference));
            Ok(Answer::KeychainDeleted)
        })
        .map(|(_, answer)| answer)
    }

    fn audit(&self, user: &User) -> Result<Answer, Reason> {
        self.verified(user, |open| {
            let Some(vault) = open.vault.as_ref() else {
                return Ok(Answer::KeychainAudit(AuditView {
                    reused_count: 0,
                    weak_count: 0,
                    reused: Vec::new(),
                    weak: Vec::new(),
                }));
            };
            let audit = vault.audit();
            Ok(Answer::KeychainAudit(AuditView {
                reused_count: audit.reused.iter().map(Vec::len).sum(),
                weak_count: audit.weak.len(),
                reused: audit
                    .reused
                    .iter()
                    .map(|group| group.iter().map(ToString::to_string).collect())
                    .collect(),
                weak: audit.weak.iter().map(ToString::to_string).collect(),
            }))
        })
        .map(|(_, answer)| answer)
    }

    /// Re-seals the keybag under the password the machine has now, given the previous one.
    fn rekey(
        &self,
        user: &User,
        uid: u32,
        previous: &Secret,
        current: &Secret,
    ) -> Result<Answer, Reason> {
        let epoch = {
            let data = lock(&user.data);
            if data.throttle.waiting() {
                return Err(Reason::Throttled);
            }
            match &data.phase {
                Phase::NeedsPrevious(learned) => {
                    // The current password must be the one the machine learned (PAM's, or Oxy's).
                    if let Some(learned) = learned {
                        let same: bool = learned
                            .password
                            .as_bytes()
                            .ct_eq(current.expose().as_bytes())
                            .into();
                        if !same {
                            drop(data);
                            lock(&user.data).throttle.record(false);
                            return Err(Reason::WrongPassword);
                        }
                    }
                }
                Phase::Locked => return Err(Reason::Locked),
                Phase::Open(_) => return Err(Reason::InvalidRequest),
            }
            data.epoch
        };
        let store = self.store(user)?;
        match store.change_password(previous.expose(), current.expose()) {
            Ok(()) => lock(&user.data).throttle.record(true),
            Err(oxyos_keychain::Error::WrongPassword) => {
                lock(&user.data).throttle.record(false);
                return Err(Reason::WrongPassword);
            }
            Err(error) => return Err(self.failed(user, "re-sealing the keybag", error)),
        }
        eprintln!(
            "oxyos-accountd: {}'s keybag was re-sealed under its new password",
            user.name
        );
        let open = self
            .open_with(user, &store, current.expose())
            .map_err(|error| self.failed(user, "unlocking", error))?;
        self.install(user, uid, epoch, open);
        Ok(self.status(user, uid))
    }

    /// Leaves the vault on this machine: its copy forgotten (with no previous password, the keybag
    /// too, made again under the password the machine has now).
    fn leave(&self, user: &User, uid: u32) -> Result<Answer, Reason> {
        let _net = lock(&user.net);
        let (epoch, learned) = {
            let mut data = lock(&user.data);
            let epoch = data.epoch;
            match &mut data.phase {
                Phase::Locked => return Err(Reason::Locked),
                Phase::Open(open) => {
                    open.forget_vault(KeychainState::Off);
                    open.creating = None;
                    open.joining = None;
                    open.recovering = None;
                    eprintln!(
                        "oxyos-accountd: {} left its vault on this machine",
                        user.name
                    );
                    return Ok(Answer::KeychainLeft);
                }
                Phase::NeedsPrevious(learned) => (epoch, learned.take()),
            }
        };
        let store = self.store(user)?;
        store
            .erase()
            .map_err(|error| self.failed(user, "forgetting the keychain", error))?;
        eprintln!(
            "oxyos-accountd: {} left its vault on this machine (its keybag's password was lost)",
            user.name
        );
        if let Some(learned) = learned {
            let open = self
                .open_with(user, &store, &learned.password)
                .map_err(|error| self.failed(user, "a new keybag", error))?;
            drop(learned);
            self.install(user, uid, epoch, open);
        } else {
            let mut data = lock(&user.data);
            if data.epoch == epoch {
                lock_data(&mut data);
            }
        }
        Ok(Answer::KeychainLeft)
    }
}

fn lock_data(data: &mut Data) {
    data.epoch += 1;
    data.phase = Phase::Locked;
    data.grace_until = None;
}

fn state_of(data: &Data) -> KeychainState {
    match &data.phase {
        Phase::Locked => KeychainState::Locked,
        Phase::NeedsPrevious(_) => KeychainState::NeedsPreviousPassword,
        Phase::Open(open) => open.state(),
    }
}

fn kind(content: &Content) -> ItemKind {
    match content {
        Content::Login(_) => ItemKind::Login,
        Content::Passkey(_) => ItemKind::Passkey,
        Content::Totp(_) => ItemKind::Totp,
        Content::Note(_) => ItemKind::Note,
        Content::Wifi(_) => ItemKind::Wifi,
    }
}

fn summary(id: &ItemId, item: &Item) -> ItemSummary {
    let (title, username, domains) = match &item.content {
        Content::Login(login) => (
            login.title.clone(),
            Some(login.username.clone()),
            login
                .urls
                .iter()
                .map(|url| Secret::new(host(url.url.expose())))
                .collect(),
        ),
        Content::Passkey(passkey) => (
            passkey.rp_name.clone(),
            Some(passkey.user_name.clone()),
            vec![passkey.rp_id.clone()],
        ),
        Content::Totp(totp) => (totp.title.clone(), Some(totp.account.clone()), Vec::new()),
        Content::Note(note) => (note.title.clone(), None, Vec::new()),
        Content::Wifi(wifi) => (wifi.ssid.clone(), None, Vec::new()),
    };
    ItemSummary {
        item_id: id.to_string(),
        kind: kind(&item.content),
        title,
        username,
        domains,
        has_code: match &item.content {
            Content::Login(login) => login.totp.is_some(),
            Content::Totp(_) => true,
            _ => false,
        },
        modified_at: item.modified_at.clone(),
    }
}

fn view(item: &Item) -> ItemView {
    let fields = match &item.content {
        Content::Login(login) => ItemFields::Login {
            title: login.title.clone(),
            username: login.username.clone(),
            password: login.password.clone(),
            urls: login.urls.iter().map(|url| url.url.clone()).collect(),
            apps: login.apps.to_vec(),
            totp: login.totp.as_ref().cloned(),
            notes: Some(login.notes.clone()),
        },
        Content::Passkey(passkey) => ItemFields::Passkey {
            rp_id: passkey.rp_id.clone(),
            rp_name: passkey.rp_name.clone(),
            user_name: passkey.user_name.clone(),
            user_display_name: passkey.user_display_name.clone(),
        },
        Content::Totp(totp) => ItemFields::Totp {
            title: totp.title.clone(),
            issuer: Some(totp.issuer.clone()),
            account: Some(totp.account.clone()),
            uri: totp.uri.clone(),
            notes: Some(totp.notes.clone()),
        },
        Content::Note(note) => ItemFields::Note {
            title: note.title.clone(),
            body: note.body.clone(),
        },
        Content::Wifi(wifi) => ItemFields::Wifi {
            ssid: wifi.ssid.clone(),
            password: Some(wifi.password.clone()),
            security: serde_json::to_value(wifi.security)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_default(),
            hidden: wifi.hidden,
        },
    };
    ItemView {
        item_id: item.id.to_string(),
        created_at: item.created_at.clone(),
        modified_at: item.modified_at.clone(),
        fields,
    }
}

fn text(value: &Option<Secret>) -> Secret {
    value.as_ref().cloned().unwrap_or_default()
}

/// The item an application's save makes, the person having said yes: the next version of the
/// application's login with the same username (the password replaced, the old one kept in its
/// history, the domain's URL added), else a new login associated with the application alone,
/// titled by the domain or the application's id. None when the login already holds all of it.
fn saved_item(open: &mut Open, login: &SaveLogin) -> Result<Option<Item>, Reason> {
    let vault = member_of(&mut open.vault, open.standing)?;
    let url = login
        .domain
        .as_ref()
        .map(|domain| format!("https://{domain}"));
    let existing = vault.open_items().find_map(|(id, item)| {
        let item = item.ok()?;
        if !vault::associated(&item, &login.desktop_entry_id) {
            return None;
        }
        match item.content {
            Content::Login(old) if old.username.expose() == login.username.expose() => {
                Some((id.clone(), old))
            }
            _ => None,
        }
    });
    let input = match existing {
        Some((id, old)) => {
            let has_url = login.domain.as_ref().is_none_or(|domain| {
                old.urls
                    .iter()
                    .any(|url| host(url.url.expose()).eq_ignore_ascii_case(domain))
            });
            if old.password.expose() == login.password.expose() && has_url {
                return Ok(None);
            }
            let mut urls: Vec<Secret> = old.urls.iter().map(|url| url.url.clone()).collect();
            if !has_url {
                urls.extend(url.map(Secret::new));
            }
            ItemInput {
                item_id: Some(id.to_string()),
                fields: ItemFields::Login {
                    title: old.title.clone(),
                    username: old.username.clone(),
                    password: Secret::new(login.password.expose()),
                    urls,
                    apps: old.apps.to_vec(),
                    totp: old.totp.as_ref().cloned(),
                    notes: (!old.notes.expose().is_empty()).then(|| old.notes.clone()),
                },
            }
        }
        None => ItemInput {
            item_id: None,
            fields: ItemFields::Login {
                title: Secret::new(
                    login
                        .domain
                        .as_deref()
                        .unwrap_or(login.desktop_entry_id.as_str()),
                ),
                username: Secret::new(login.username.expose()),
                password: Secret::new(login.password.expose()),
                urls: url.into_iter().map(Secret::new).collect(),
                apps: vec![Secret::new(login.desktop_entry_id.as_str())],
                totp: None,
                notes: None,
            },
        },
    };
    build(vault, &input).map(Some)
}

/// The item Settings' input makes: a new one, or the next version of the one it names (its
/// creation kept, its counter moved on, a changed password kept in its history).
fn build(vault: &Vault, input: &ItemInput) -> Result<Item, Reason> {
    let now = oxyos_keychain::wire::now();
    let (id, created_at, previous) = match &input.item_id {
        Some(text) => {
            let id = ItemId::parse(text).map_err(|_| Reason::InvalidRequest)?;
            let previous = vault
                .open_item(&id)
                .ok()
                .flatten()
                .ok_or(Reason::NotFound)?;
            (id, previous.created_at.clone(), Some(previous))
        }
        None => (
            ItemId::random().map_err(|_| Reason::Failed)?,
            now.clone(),
            None,
        ),
    };
    // Timestamps written as JavaScript writes them order as their times.
    let modified_at = if now >= created_at {
        now.clone()
    } else {
        created_at.clone()
    };
    if let Some(previous) = &previous {
        let same_kind = matches!(
            (&previous.content, &input.fields),
            (Content::Login(_), ItemFields::Login { .. })
                | (Content::Totp(_), ItemFields::Totp { .. })
                | (Content::Note(_), ItemFields::Note { .. })
                | (Content::Wifi(_), ItemFields::Wifi { .. })
        );
        if !same_kind {
            return Err(Reason::InvalidRequest);
        }
    }
    let content = match &input.fields {
        ItemFields::Login {
            title,
            username,
            password,
            urls,
            apps,
            totp,
            notes,
        } => {
            let mut history = Vec::new();
            if let Some(Item {
                content: Content::Login(old),
                ..
            }) = &previous
            {
                history = old.password_history.clone();
                if !old.password.is_empty() && old.password.expose() != password.expose() {
                    history.insert(
                        0,
                        PreviousPassword {
                            changed_at: modified_at.clone(),
                            password: old.password.clone(),
                        },
                    );
                }
                history.truncate(PASSWORD_HISTORY);
            }
            if apps.iter().any(|app| !valid_entry_id(app.expose())) {
                return Err(Reason::InvalidRequest);
            }
            Content::Login(Login {
                title: title.clone(),
                username: username.clone(),
                password: password.clone(),
                urls: urls.iter().map(|url| Url { url: url.clone() }).collect(),
                apps: apps.to_vec(),
                totp: totp.as_ref().cloned(),
                notes: text(notes),
                password_history: history,
            })
        }
        ItemFields::Passkey { .. } => return Err(Reason::InvalidRequest),
        ItemFields::Totp {
            title,
            issuer,
            account,
            uri,
            notes,
        } => Content::Totp(TotpItem {
            title: title.clone(),
            issuer: text(issuer),
            account: text(account),
            uri: uri.clone(),
            notes: text(notes),
        }),
        ItemFields::Note { title, body } => Content::Note(Note {
            title: title.clone(),
            body: body.clone(),
        }),
        ItemFields::Wifi {
            ssid,
            password,
            security,
            hidden,
        } => Content::Wifi(Wifi {
            ssid: ssid.clone(),
            password: text(password),
            security: serde_json::from_value::<WifiSecurity>(serde_json::Value::String(
                security.clone(),
            ))
            .map_err(|_| Reason::InvalidRequest)?,
            hidden: *hidden,
        }),
    };
    let item = Item {
        counter: vault.next_counter(&id),
        id,
        created_at,
        modified_at,
        content,
    };
    item.validate().map_err(|_| Reason::InvalidRequest)?;
    Ok(item)
}

impl<V: VaultApi + 'static> Keychains for KeychainService<V> {
    fn unlock(&self, uid: u32, user: &str, password: &Secret) -> KeychainState {
        let account = self.user(uid, user);
        let epoch = {
            let data = lock(&account.data);
            if matches!(data.phase, Phase::Open(_)) {
                return state_of(&data);
            }
            data.epoch
        };
        // The keybag's Argon2id, without the keychain's state held.
        let Ok(store) = self.store(&account) else {
            return KeychainState::Locked;
        };
        match self.open_with(&account, &store, password.expose()) {
            Ok(open) => {
                eprintln!("oxyos-accountd: {user}'s keychain unlocked");
                self.install(&account, uid, epoch, open)
            }
            Err(oxyos_keychain::Error::WrongPassword) => {
                eprintln!(
                    "oxyos-accountd: {user}'s keybag does not open with the account's password \
                     (changed elsewhere): Settings asks for the previous one"
                );
                let learned = self.learned(&account, password.expose());
                let mut data = lock(&account.data);
                if data.epoch == epoch {
                    data.epoch += 1;
                    data.phase = Phase::NeedsPrevious(learned);
                    data.grace_until = (data.sessions == 0).then(|| Instant::now() + GRACE);
                }
                state_of(&data)
            }
            Err(error) => {
                eprintln!("oxyos-accountd: {user}'s keychain stays locked: {error}");
                KeychainState::Locked
            }
        }
    }

    fn session_opened(&self, uid: u32, user: &str) {
        let account = self.user(uid, user);
        let mut data = lock(&account.data);
        data.sessions += 1;
        data.grace_until = None;
    }

    fn session_closed(&self, uid: u32, remaining: usize) {
        let Some(account) = lock(&self.users).get(&uid).cloned() else {
            return;
        };
        let mut data = lock(&account.data);
        data.sessions = remaining;
        if remaining == 0 && !matches!(data.phase, Phase::Locked) {
            eprintln!(
                "oxyos-accountd: {}'s session ended: its keychain locked",
                account.name
            );
            lock_data(&mut data);
        }
    }

    fn lock(&self, uid: u32) {
        if let Some(account) = lock(&self.users).get(&uid).cloned() {
            let mut data = lock(&account.data);
            if !matches!(data.phase, Phase::Locked) {
                eprintln!("oxyos-accountd: {}'s keychain locked", account.name);
            }
            lock_data(&mut data);
        }
    }

    fn registrar(&self, uid: u32, user: &str, request: Request) -> Answer {
        let account = self.user(uid, user);
        match request {
            Request::KeychainLock {} => {
                Keychains::lock(self, uid);
                Answer::KeychainLocked
            }
            Request::AutofillGenerate {} => answer(
                self.with_open(&account, None, |_| {
                    let password =
                        oxyos_keychain::password::generate().map_err(|_| Reason::Failed)?;
                    Ok(Answer::GeneratedPassword(password.clone()))
                })
                .map(|(_, answer)| answer),
            ),
            Request::AutofillSuggest {
                desktop_entry_id,
                field,
            } => {
                if !valid_entry_id(&desktop_entry_id) {
                    return refused(Reason::InvalidRequest);
                }
                answer(
                    self.with_open(&account, None, |open| {
                        self.suggest(open, &desktop_entry_id, field)
                    })
                    .map(|(_, answer)| answer),
                )
            }
            Request::AutofillFill {
                desktop_entry_id,
                reference,
                field,
            } => {
                if !valid_entry_id(&desktop_entry_id) {
                    return refused(Reason::InvalidRequest);
                }
                answer(
                    self.with_open(&account, None, |open| {
                        Self::fill(open, &desktop_entry_id, &reference, field)
                    })
                    .map(|(_, answer)| answer),
                )
            }
            _ => refused(Reason::NotPermitted),
        }
    }

    fn settings(&self, uid: u32, user: &str, request: Request) -> Answer {
        let account = self.user(uid, user);
        let account = &*account;
        let item = |text: &str| ItemId::parse(text).map_err(|_| Reason::InvalidRequest);
        let enrollment = |text: &str| EnrollmentId::parse(text).map_err(|_| Reason::InvalidRequest);
        answer(match request {
            Request::KeychainStatus {} => Ok(self.status(account, uid)),
            Request::KeychainCreate {} => self.create(account, uid),
            Request::KeychainConfirmRecoveryKey { recovery_key } => {
                self.confirm_recovery_key(account, uid, &recovery_key)
            }
            Request::KeychainJoin {
                method,
                recovery_key,
            } => self.join(account, uid, method, recovery_key.as_ref()),
            Request::KeychainEnrollments {} => self.enrollments(account, uid),
            Request::KeychainApprove {
                enrollment_id,
                confirm,
            } => enrollment(&enrollment_id).and_then(|id| self.approve(account, uid, &id, confirm)),
            Request::KeychainDeny { enrollment_id } => {
                enrollment(&enrollment_id).and_then(|id| self.deny(account, uid, &id))
            }
            Request::KeychainMembers {} => self.members(account, uid),
            Request::KeychainRemoveMember { member_id } => MemberId::parse(&member_id)
                .map_err(|_| Reason::InvalidRequest)
                .and_then(|id| self.remove_member(account, uid, &id)),
            Request::KeychainVerify { password } => self.verify(account, &password),
            Request::KeychainList { query, kind } => self.list(account, query.as_ref(), kind),
            Request::KeychainReveal { item_id } => {
                item(&item_id).and_then(|id| self.reveal(account, &id))
            }
            Request::KeychainPut { item } => self.put(account, uid, &item),
            Request::KeychainDelete { item_id } => {
                item(&item_id).and_then(|id| self.delete(account, uid, &id))
            }
            Request::KeychainAudit {} => self.audit(account),
            Request::KeychainRekey { previous, current } => {
                self.rekey(account, uid, &previous, &current)
            }
            Request::KeychainLeave {} => self.leave(account, uid),
            Request::KeychainGenerate {} => self.generate(account),
            Request::KeychainCode { item_id } => {
                item(&item_id).and_then(|id| self.code(account, &id))
            }
            _ => Err(Reason::NotPermitted),
        })
    }

    fn may_save(&self, uid: u32, user: &str) -> Result<(), Reason> {
        let account = self.user(uid, user);
        self.with_open(&account, None, |open| {
            member_of(&mut open.vault, open.standing).map(|_| ())
        })
        .map(|_| ())
    }

    fn save(&self, uid: u32, user: &str, login: &SaveLogin) -> Result<(), Reason> {
        if !valid_entry_id(&login.desktop_entry_id) {
            return Err(Reason::InvalidRequest);
        }
        let account = self.user(uid, user);
        self.save_login(&account, uid, login)
    }

    fn passkey(&self, uid: u32, user: &str, asker: &Asker<'_>, request: Request) -> Answer {
        let account = self.user(uid, user);
        let answer = match &request {
            Request::PasskeyCreate(create) => self.passkey_create(&account, uid, asker, create),
            Request::PasskeyGet(get) => self.passkey_get(&account, asker, get),
            _ => Err(Reason::NotPermitted),
        };
        if let Err(reason) = &answer {
            eprintln!(
                "oxyos-accountd: {}'s passkey for {}: refused {reason:?}",
                account.name, asker.desktop_entry_id
            );
        }
        self::answer(answer)
    }

    fn serve_prompts(&self, uid: u32, user: &str, connection: &Connection) -> std::io::Result<()> {
        let account = self.user(uid, user);
        self.serve_prompts_of(&account, connection)
    }

    fn forget(&self, uid: u32, user: &str) {
        Keychains::lock(self, uid);
        lock(&self.users).remove(&uid);
        let directory = self.directory.join(user);
        match std::fs::remove_dir_all(&directory) {
            Ok(()) => eprintln!("oxyos-accountd: {user}'s keychain was forgotten"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                eprintln!("oxyos-accountd: {user}'s keychain cannot be forgotten: {error}")
            }
        }
    }
}
