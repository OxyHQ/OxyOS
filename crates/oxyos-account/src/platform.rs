//! The machine's Oxy session, the platform's ([ADR 0020](../../docs/adr/0020-oxy-account-login.mdx),
//! decisions 2, 3, 5 and 6; Oxy ADR 0058): what `oxyos-accountd` does online for enrolled
//! accounts once they are set up.
//!
//! - **The online checks** ([`Platform::check`], step 3). A password the machine rejects is asked
//!   of Oxy: accepted, the verifier is replaced with it (the password was changed elsewhere).
//!   A password the machine accepts is accepted at once and asked of Oxy afterwards, never delaying
//!   the session: refused, the account is flagged `passwordChanged`, and while flagged and online
//!   a locally accepted password waits for Oxy (the old one works only offline). Each online
//!   check is the platform's own sign-in, which rotates the platform credential: the new one,
//!   once Oxy confirms it (ADR 0058 R8; until then it is pending and the old one works),
//!   replaces the old in the account's file, written whole.
//! - **Revocation.** Oxy answering that the device's session is gone (a mint refused with
//!   `invalid_device_secret`, `no_active_session` or `account_not_on_device`, R4) signs the account
//!   out everywhere on the machine: its credential forgotten, its verifier locked, its sessions
//!   ended. It lists as `signInAgain`; setting it up again online keeps its user and home.
//! - **Applications' tokens** ([`Platform::token`], decision 6): minted for one application with
//!   the platform credential (`/session/device/app-token`), cached per account and application,
//!   minted again about a minute before they expire, or at once when an application says the one
//!   it has was refused. Offline, none: the snapshot still says signed in.
//! - **The snapshot** ([`Platform::snapshot`]) applications see: the SDK's `OxyHostSnapshot`.
//!   Subscribers are woken for every change ([`Platform::watch`]), but its revision changes only
//!   when its status or its account does (signed in, signed out, removed, another account): the
//!   SDK takes a new revision for a switch of account, so a profile read anew or Oxy reached or
//!   not is told with the same revision.
//! - **Mints are rationed** per account and application: after a refusal or offline none is tried
//!   for a while, and at most [`MAX_MINTS`] in [`MINT_WINDOW`] (Oxy allows twelve), however often
//!   an application asks.
//! - **Removing an account** ([`Platform::remove`], decision 5): its device ended online when Oxy
//!   can be reached, its verifier locked and credential forgotten at once; then its sessions
//!   ended, its user and home removed, its file and picture forgotten.
//!
//! - **Erasing the machine** ([ADR 0054](../../docs/adr/0054-recovery-and-erasing.mdx), the
//!   [`erase`](crate::erase) module's): once the erase is armed the platform mints no token and
//!   checks no password online ([`Platform::begin_erasing`]), and every account is signed out of
//!   this device online as removing one does ([`Platform::sign_out_for_erase`]).
//!
//! Every change of an account's file happens under the machine's lock ([`Setup::lock_machine`])
//! and only if the file still holds the credential the change was made from, so a setup, a
//! rotation and a revocation never undo one another. An account's online calls are made one at a
//! time.

use crate::api::{ApiError, Attestation, DeviceCredential, OxyApi, Profile, SignInStep};
use crate::secret::Secret;
use crate::setup::Setup;
use crate::store::{Enrollment, State, ERASE_ARMED};
use crate::users::{verifier_password, Users, UsersError};
use crate::{AccountId, UserName};
use rustix::fd::OwnedFd;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, SystemTime};
// The messages, the protocol's own crate's (`oxyos-account-proto`): clients link them alone.
pub use oxyos_account_proto::platform::{
    CheckReason, Checked, Local, LocalView, Snapshot, SnapshotAccount, Status, Token, TokenReason,
    Verdict,
};

/// A cached token is minted again this long before it expires.
pub const REFRESH_BEFORE: Duration = Duration::from_secs(60);

/// At most this many mints per account and application in [`MINT_WINDOW`].
pub const MAX_MINTS: usize = 10;
pub const MINT_WINDOW: Duration = Duration::from_secs(15 * 60);
/// After a mint Oxy refused, none for that application is tried for this long.
pub const REFUSED_HOLDOFF: Duration = Duration::from_secs(60);
/// After a mint that found Oxy unreachable, none for that application is tried for this long.
pub const OFFLINE_HOLDOFF: Duration = Duration::from_secs(15);

/// How long removing an account waits for its user's processes to end before `userdel` gives up.
const REMOVE_WAIT: Duration = if cfg!(test) {
    Duration::from_secs(1)
} else {
    Duration::from_secs(15)
};

/// What removing an account came to.
#[derive(Debug, PartialEq, Eq)]
pub enum RemoveError {
    /// No enrolled account runs as that user.
    NotEnrolled,
    /// The account's file could not be changed: nothing was removed.
    Failed(String),
}

/// What the platform knows of an account beyond its file.
struct Live {
    /// The account's online calls, one at a time.
    online_calls: Arc<Mutex<()>>,
    tokens: HashMap<Attestation, Cached>,
    /// Whether the last call to Oxy reached it.
    online: bool,
    /// The snapshot's revision: that of the last change of its status or account.
    revision: u64,
    /// The status and account the revision is for, once a snapshot was made.
    identity: Option<(Status, Option<String>)>,
    /// No mint for an application before this (it was refused, or Oxy was unreachable).
    holdoff: HashMap<Attestation, SystemTime>,
    /// Each application's mints in the last [`MINT_WINDOW`].
    mints: HashMap<Attestation, Vec<SystemTime>>,
}

struct Cached {
    token: Secret,
    expires_at: String,
    expires: SystemTime,
}

/// What became of an online sign-in with the platform credential.
enum Online {
    /// Oxy accepted the password; the credential was rotated.
    Accepted,
    /// The password the machine already knows is right, and Oxy asks the authenticator's code,
    /// which a check cannot answer: the credential is unchanged.
    PasswordRight,
    /// A password the machine does not know proves only the first factor: it never becomes the
    /// verifier without the authenticator's code (setting the account up again asks for it).
    SecondFactorRequired,
    /// Oxy refused the password.
    WrongPassword,
    /// Oxy cannot be reached.
    Offline,
    /// The device's session is gone: the account was signed out.
    Revoked,
    /// Anything else (a lockout, an answer outside the contract): no judgement on the password.
    Failed(String),
}

impl Online {
    /// Says a failure in the service's log (never a secret: what Oxy or the machine said).
    fn logged(self, user: &str) -> Self {
        if let Online::Failed(why) = &self {
            eprintln!("oxyos-accountd: {user}: the online check failed: {why}");
        }
        self
    }
}

pub struct Platform<A, U> {
    setup: Setup<A, U>,
    live: Mutex<HashMap<String, Live>>,
    /// Revisions start at the service's start in milliseconds, so they still rise across a restart.
    revision: AtomicU64,
    watchers: Mutex<Vec<(u32, Weak<OwnedFd>)>>,
    /// Checks still asking Oxy after their answer was given, for [`Platform::settle`].
    background: Arc<(Mutex<usize>, Condvar)>,
    /// The accounts' keychains (ADR 0050), when the service runs them.
    keychains: std::sync::OnceLock<Arc<dyn crate::keychain::Keychains>>,
    /// The machine's erase is armed: nothing more is asked of Oxy but its accounts' sign-outs.
    erasing: AtomicBool,
    /// The accounts' PINs' checks and their waits (`crate::pin`).
    pins: crate::pin::PinChecks,
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

/// When a token expires, from its `expiresAt` (RFC 3339).
fn expiry(expires_at: &str) -> Option<SystemTime> {
    let timestamp: jiff::Timestamp = expires_at.parse().ok()?;
    let seconds = u64::try_from(timestamp.as_second()).ok()?;
    Some(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
}

fn same(a: &DeviceCredential, b: &DeviceCredential) -> bool {
    a.device_id == b.device_id && a.device_secret == b.device_secret
}

fn copy(credential: &DeviceCredential) -> DeviceCredential {
    DeviceCredential {
        device_id: credential.device_id.clone(),
        device_secret: Secret::new(credential.device_secret.expose()),
    }
}

/// Settings and the shell are OxyOS itself: their tokens are the platform's own application's.
pub const OWN_APPLICATIONS: [&str; 2] = ["so.oxy.settings", "so.oxy.shell"];

/// What a registered application's tokens are minted as: OxyOS itself for Settings and the shell
/// registered as system applications, else its desktop entry id.
pub fn attestation(desktop_entry_id: &str, system: bool) -> Attestation {
    if system && OWN_APPLICATIONS.contains(&desktop_entry_id) {
        Attestation::OwnPlatform
    } else {
        Attestation::Entry(desktop_entry_id.to_owned())
    }
}

/// How the person proves themselves to change their local sign-in.
#[derive(Debug)]
pub enum Proof {
    Password(Secret),
    Pin(Secret),
}

/// A change to an account's local sign-in (Settings').
#[derive(Debug)]
pub enum LocalChange {
    SetPin(Secret),
    RemovePin,
    PasswordSignIn(bool),
}

impl LocalChange {
    /// What it is, for the service's log (never the PIN).
    pub fn name(&self) -> &'static str {
        match self {
            LocalChange::SetPin(_) => "setPin",
            LocalChange::RemovePin => "removePin",
            LocalChange::PasswordSignIn(true) => "passwordOn",
            LocalChange::PasswordSignIn(false) => "passwordOff",
        }
    }
}

/// Why a change to the local sign-in was refused; nothing changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalRefusal {
    WrongPassword,
    WrongPin,
    /// Too many wrong attempts: another after this many seconds.
    Throttled(u64),
    /// The PIN is locked out: the password, or Oxy online, first.
    PinLocked,
    /// Not 4 to 16 digits, or the password itself.
    UnusablePin,
    /// The password does not sign in here: it proves nothing, and the PIN cannot go.
    PasswordSignInOff,
    /// The password cannot be turned off without a PIN.
    NoPin,
    /// Turning the password on asks Oxy, which cannot be reached.
    Offline,
    /// The account has an authenticator: turning the password on is setting it up again.
    SecondFactorRequired,
    Failed,
}

impl<A: OxyApi + Send + Sync + 'static, U: Users + Send + Sync + 'static> Platform<A, U> {
    pub fn new(setup: Setup<A, U>) -> Self {
        // An erase armed before the service started again still holds: the machine is erased at
        // its next boot, and nothing more is asked of Oxy until then.
        let erasing = setup.store().fact(ERASE_ARMED);
        Self {
            setup,
            live: Mutex::new(HashMap::new()),
            revision: AtomicU64::new(now_millis()),
            watchers: Mutex::new(Vec::new()),
            background: Arc::new((Mutex::new(0), Condvar::new())),
            keychains: std::sync::OnceLock::new(),
            erasing: AtomicBool::new(erasing),
            pins: crate::pin::PinChecks::default(),
        }
    }

    /// Gives the platform the accounts' keychains (once).
    pub fn set_keychains(&self, keychains: Arc<dyn crate::keychain::Keychains>) {
        let _ = self.keychains.set(keychains);
    }

    /// The accounts' keychains, when the service runs them.
    pub fn keychains(&self) -> Option<&Arc<dyn crate::keychain::Keychains>> {
        self.keychains.get()
    }

    pub fn setup(&self) -> &Setup<A, U> {
        &self.setup
    }

    fn with_live<T>(&self, user: &str, f: impl FnOnce(&mut Live) -> T) -> T {
        let mut live = self.live.lock().unwrap_or_else(|p| p.into_inner());
        let base = self.revision.load(Ordering::SeqCst);
        let entry = live.entry(user.to_owned()).or_insert_with(|| Live {
            online_calls: Arc::new(Mutex::new(())),
            tokens: HashMap::new(),
            online: false,
            revision: base,
            identity: None,
            holdoff: HashMap::new(),
            mints: HashMap::new(),
        });
        f(entry)
    }

    /// Every enrolled account's picture read where it is missing or out of date (the service
    /// started; accounts set up before pictures were kept): online, each one's subscribers told.
    /// Run on a thread of its own: nothing waits for Oxy's cloud.
    pub fn refresh_pictures(&self) {
        let Ok(enrollments) = self.setup.store().enrollments() else {
            return;
        };
        for enrollment in enrollments {
            if enrollment.state() == crate::store::State::Ready
                && self
                    .setup
                    .picture_user(&enrollment.user, &enrollment.profile)
            {
                self.changed(&enrollment);
            }
        }
    }

    /// The account was signed out or removed: a new revision, whatever snapshots were made between.
    fn switched(&self, enrollment: &Enrollment) {
        let revision = self.revision.fetch_add(1, Ordering::SeqCst) + 1;
        self.with_live(enrollment.user.as_str(), |live| {
            live.revision = revision;
            live.identity = None;
        });
        self.changed(enrollment);
    }

    /// Something the snapshot says changed: its subscribers are woken (its revision is the
    /// snapshot's to decide).
    fn changed(&self, enrollment: &Enrollment) {
        let mut watchers = self.watchers.lock().unwrap_or_else(|p| p.into_inner());
        watchers.retain(|(uid, waker)| match waker.upgrade() {
            Some(waker) => {
                if *uid == enrollment.uid {
                    let _ = rustix::io::write(&*waker, &1u64.to_ne_bytes());
                }
                true
            }
            None => false,
        });
    }

    /// Whether Oxy was reached, as the last call found; a change is a new snapshot.
    fn reached(&self, enrollment: &Enrollment, online: bool) {
        let was = self.with_live(enrollment.user.as_str(), |live| {
            std::mem::replace(&mut live.online, online)
        });
        if was != online {
            self.changed(enrollment);
        }
    }

    fn online_calls(&self, user: &str) -> Arc<Mutex<()>> {
        self.with_live(user, |live| live.online_calls.clone())
    }

    /// A subscriber of `uid`'s account: `waker` (an eventfd) is written to on every change.
    pub fn watch(&self, uid: u32, waker: &Arc<OwnedFd>) {
        self.watchers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push((uid, Arc::downgrade(waker)));
    }

    /// The enrolled account whose user has `uid`.
    pub fn enrollment_of(&self, uid: u32) -> std::io::Result<Option<Enrollment>> {
        self.setup.store().get_uid(uid)
    }

    /// The snapshot of `uid`'s account.
    pub fn snapshot(&self, uid: u32) -> Snapshot {
        let enrollment = match self.enrollment_of(uid) {
            Ok(Some(enrollment)) => enrollment,
            Ok(None) => {
                return Snapshot {
                    status: Status::SignedOut,
                    revision: self.revision.load(Ordering::SeqCst),
                    account: None,
                    online: false,
                }
            }
            Err(error) => {
                eprintln!("oxyos-accountd: uid {uid}'s account cannot be read: {error}");
                return Snapshot {
                    status: Status::Unavailable,
                    revision: self.revision.load(Ordering::SeqCst),
                    account: None,
                    online: false,
                };
            }
        };
        let status = match enrollment.state() {
            State::Ready => Status::SignedIn,
            State::SignInAgain => Status::SignedOut,
        };
        let identity = (
            status,
            (status == Status::SignedIn).then(|| enrollment.account.as_str().to_owned()),
        );
        let (revision, online) = self.with_live(enrollment.user.as_str(), |live| {
            if live.identity.as_ref() != Some(&identity) {
                if live.identity.is_some() {
                    live.revision = self.revision.fetch_add(1, Ordering::SeqCst) + 1;
                }
                live.identity = Some(identity);
            }
            (live.revision, live.online)
        });
        if enrollment.state() == State::SignInAgain {
            return Snapshot {
                status: Status::SignedOut,
                revision,
                account: None,
                online,
            };
        }
        let profile = &enrollment.profile;
        Snapshot {
            status: Status::SignedIn,
            revision,
            account: Some(SnapshotAccount {
                id: enrollment.account.as_str().to_owned(),
                handle: profile.username.clone().unwrap_or_default(),
                display_name: profile.display_name.clone(),
                avatar: profile.avatar.clone(),
                picture: self
                    .setup
                    .store()
                    .picture(&enrollment.user, profile.avatar.as_deref())
                    .map(|path| path.to_string_lossy().into_owned()),
                languages: profile.languages.clone(),
            }),
            online,
        }
    }

    /// A token for `attestation`'s application, for `uid`'s account: the cached one while it has
    /// more than [`REFRESH_BEFORE`] left (unless `reason` says it was refused), else a new one.
    /// None offline, signed out, or refused.
    pub fn token(&self, uid: u32, attestation: &Attestation, reason: TokenReason) -> Option<Token> {
        if self.erasing() {
            return None;
        }
        let enrollment = self.enrollment_of(uid).ok().flatten()?;
        let user = enrollment.user.as_str().to_owned();
        let calls = self.online_calls(&user);
        let _calls = calls.lock().unwrap_or_else(|p| p.into_inner());
        // The file as it is now, under the account's calls: a rotation may just have replaced it.
        let enrollment = self.setup.store().get(&enrollment.account).ok().flatten()?;
        let credential = enrollment.credential.as_ref()?;
        let now = SystemTime::now();
        if reason != TokenReason::Response401 {
            let cached = self.with_live(&user, |live| {
                live.tokens.get(attestation).and_then(|cached| {
                    (cached.expires > now + REFRESH_BEFORE).then(|| Token {
                        access_token: Secret::new(cached.token.expose()),
                        expires_at: cached.expires_at.clone(),
                    })
                })
            });
            if cached.is_some() {
                return cached;
            }
        }
        // Rationed: not while held off after a failure, and at most MAX_MINTS in the window (the
        // cached token, if any, stays for the next ordinary request).
        let may = self.with_live(&user, |live| {
            if live
                .holdoff
                .get(attestation)
                .is_some_and(|until| *until > now)
            {
                return false;
            }
            let recent = live.mints.entry(attestation.clone()).or_default();
            recent.retain(|at| now.duration_since(*at).is_ok_and(|age| age < MINT_WINDOW));
            if recent.len() >= MAX_MINTS {
                return false;
            }
            recent.push(now);
            true
        });
        if !may {
            return None;
        }
        self.with_live(&user, |live| live.tokens.remove(attestation));
        let hold = |wait: Duration| {
            self.with_live(&user, |live| {
                live.holdoff.insert(attestation.clone(), now + wait);
            })
        };
        match self
            .setup
            .api()
            .mint_app_token(credential, &enrollment.account, attestation)
        {
            Ok(minted) => {
                self.reached(&enrollment, true);
                self.with_live(&user, |live| live.holdoff.remove(attestation));
                if minted.account_id != enrollment.account.as_str() {
                    eprintln!("oxyos-accountd: Oxy minted a token for another account: dropped");
                    return None;
                }
                if let Some(expires) = expiry(&minted.expires_at) {
                    self.with_live(&user, |live| {
                        live.tokens.insert(
                            attestation.clone(),
                            Cached {
                                token: Secret::new(minted.access_token.expose()),
                                expires_at: minted.expires_at.clone(),
                                expires,
                            },
                        )
                    });
                }
                Some(Token {
                    access_token: minted.access_token,
                    expires_at: minted.expires_at,
                })
            }
            Err(ApiError::Offline(_)) => {
                hold(OFFLINE_HOLDOFF);
                self.reached(&enrollment, false);
                None
            }
            Err(ApiError::InvalidDeviceSecret | ApiError::AccountNotOnDevice) => {
                self.reached(&enrollment, true);
                self.revoked(&enrollment, credential);
                None
            }
            Err(error) => {
                hold(REFUSED_HOLDOFF);
                self.reached(&enrollment, true);
                eprintln!("oxyos-accountd: {user}: no token for {attestation:?}: {error}");
                None
            }
        }
    }

    /// The online check of `user`'s password (ADR 0020 decision 2, step 3), after PAM said
    /// `local`. A password that stands forgets the account's wrong PINs, and signs a locked-out
    /// PIN in again. An account whose password does not sign in on this computer (Settings
    /// turned it off, its PIN its one local credential) is refused here, and Oxy is not asked: a
    /// password Oxy accepts never turns it on again behind the person's back.
    pub fn check(self: &Arc<Self>, user: &str, password: Secret, local: Local) -> Checked {
        let store = self.setup.store();
        if let Ok(Some(enrollment)) = store.get_user(user) {
            // A local sign-in that cannot be read is no reason to let a password in.
            if !store
                .local(&enrollment.user)
                .is_ok_and(|local| local.password)
            {
                return Checked::REJECTED;
            }
            let checked = self.check_password(user, password, local);
            if checked.result == Verdict::Accepted {
                let _machine = self.setup.lock_machine();
                if let Err(error) = self.pins.forget_failures(store, &enrollment.user) {
                    eprintln!("oxyos-accountd: {user}'s wrong PINs cannot be forgotten: {error}");
                }
            }
            return checked;
        }
        self.check_password(user, password, local)
    }

    /// `user`'s PIN (`crate::pin`): checked here, never online, its wrong attempts counted.
    /// Anything but an enrolled account's user has no PIN; a file that cannot be read checks
    /// nothing.
    #[cfg(feature = "local")]
    pub fn check_pin(&self, user: &str, pin: &Secret) -> crate::pin::PinCheck {
        let store = self.setup.store();
        let _machine = self.setup.lock_machine();
        let enrollment = match store.get_user(user) {
            Ok(Some(enrollment)) => enrollment,
            Ok(None) => return crate::pin::PinCheck::NoPin,
            Err(error) => {
                eprintln!("oxyos-accountd: {user}'s account cannot be read: {error}");
                return crate::pin::PinCheck::NoPin;
            }
        };
        if enrollment.state() == State::SignInAgain {
            return crate::pin::PinCheck::NoPin;
        }
        match self
            .pins
            .check(store, &enrollment.user, pin, std::time::Instant::now())
        {
            Ok(checked) => checked,
            Err(error) => {
                eprintln!("oxyos-accountd: {user}'s PIN cannot be checked: {error}");
                crate::pin::PinCheck::NoPin
            }
        }
    }

    /// The PIN checks.
    pub fn pins(&self) -> &crate::pin::PinChecks {
        &self.pins
    }

    /// `uid`'s account's local sign-in, as Settings and the lock screen see it.
    pub fn local_sign_in(&self, uid: u32) -> std::io::Result<Option<LocalView>> {
        let Some(enrollment) = self.enrollment_of(uid)? else {
            return Ok(None);
        };
        let local = self.setup.store().local(&enrollment.user)?;
        Ok(Some(local_view(&local)))
    }

    /// Settings' change to `uid`'s account's local sign-in, the person proved by `proof`
    /// (`crate::pin`):
    ///
    /// - **a PIN set or changed**: 4 to 16 digits, never the password the person just proved;
    /// - **the PIN removed**, only while the password signs in (one local credential stays);
    /// - **the password turned off**, only while a PIN exists: the user's verifier is locked, and
    ///   the PIN is the account's one local credential;
    /// - **the password turned on**, proved by the password itself, checked with Oxy online (the
    ///   machine has no verifier to check it with): it becomes the verifier again.
    ///
    /// A password proof is checked by PAM against the verifier (`passwords`, throttled), a PIN
    /// proof as a PIN is, each wrong one counted. Nothing changes on a refusal.
    #[cfg(feature = "local")]
    pub fn change_local(
        self: &Arc<Self>,
        uid: u32,
        proof: Proof,
        change: LocalChange,
        passwords: &dyn crate::erase::Passwords,
    ) -> Result<LocalView, LocalRefusal> {
        use crate::erase::PasswordCheck;
        use crate::pin::{hash_pin, usable_pin, PinCheck, PinRecord};
        let enrollment = self
            .enrollment_of(uid)
            .map_err(|_| LocalRefusal::Failed)?
            .ok_or(LocalRefusal::Failed)?;
        let store = self.setup.store();
        let user = enrollment.user.clone();
        let local = store.local(&user).map_err(|_| LocalRefusal::Failed)?;
        if let LocalChange::SetPin(pin) = &change {
            if !usable_pin(pin) {
                return Err(LocalRefusal::UnusablePin);
            }
            if let Proof::Password(password) = &proof {
                if password.expose() == pin.expose() {
                    return Err(LocalRefusal::UnusablePin);
                }
            }
        }
        // The person, proved.
        match (&proof, &change) {
            (Proof::Password(password), LocalChange::PasswordSignIn(true)) => {
                if local.password {
                    return Ok(local_view(&local));
                }
                match self
                    .sign_in(&enrollment, password, true)
                    .logged(user.as_str())
                {
                    Online::Accepted => {}
                    Online::WrongPassword => return Err(LocalRefusal::WrongPassword),
                    Online::Offline => return Err(LocalRefusal::Offline),
                    Online::SecondFactorRequired | Online::PasswordRight => {
                        return Err(LocalRefusal::SecondFactorRequired)
                    }
                    Online::Revoked | Online::Failed(_) => return Err(LocalRefusal::Failed),
                }
            }
            (Proof::Pin(_), LocalChange::PasswordSignIn(true)) => {
                return Err(LocalRefusal::WrongPassword)
            }
            (Proof::Password(password), _) => {
                if !local.password {
                    return Err(LocalRefusal::PasswordSignInOff);
                }
                match passwords.check(&user, Secret::new(password.expose())) {
                    PasswordCheck::Accepted => {}
                    PasswordCheck::Rejected => return Err(LocalRefusal::WrongPassword),
                    PasswordCheck::Throttled(wait) => {
                        return Err(LocalRefusal::Throttled(wait.as_secs().max(1)))
                    }
                    PasswordCheck::Failed(_) => return Err(LocalRefusal::Failed),
                }
            }
            (Proof::Pin(pin), _) => match self.check_pin(user.as_str(), pin) {
                PinCheck::Accepted => {}
                PinCheck::Rejected { locked: true, .. } | PinCheck::Locked => {
                    return Err(LocalRefusal::PinLocked)
                }
                PinCheck::Rejected { .. } | PinCheck::NoPin => return Err(LocalRefusal::WrongPin),
                PinCheck::Throttled(wait) => {
                    return Err(LocalRefusal::Throttled(wait.as_secs().max(1)))
                }
            },
        }
        let verifier = match &change {
            LocalChange::SetPin(pin) => Some(hash_pin(pin).map_err(|_| LocalRefusal::Failed)?),
            _ => None,
        };
        let _machine = self.setup.lock_machine();
        let before = store.local(&user).map_err(|_| LocalRefusal::Failed)?;
        let mut after = before.clone();
        // A password proved signs a locked-out PIN in again.
        if let (Proof::Password(_), Some(record)) = (&proof, after.pin.as_mut()) {
            record.failures = 0;
        }
        match change {
            LocalChange::SetPin(_) => {
                after.pin = verifier.map(|verifier| PinRecord {
                    verifier,
                    failures: 0,
                });
            }
            LocalChange::RemovePin => {
                if !after.password {
                    return Err(LocalRefusal::PasswordSignInOff);
                }
                after.pin = None;
            }
            LocalChange::PasswordSignIn(false) => {
                if after.pin.is_none() {
                    return Err(LocalRefusal::NoPin);
                }
                after.password = false;
            }
            LocalChange::PasswordSignIn(true) => after.password = true,
        }
        store
            .put_local(&user, &after)
            .map_err(|_| LocalRefusal::Failed)?;
        if before.password && !after.password {
            // The verifier no longer opens the account; if it cannot be locked, nothing changed.
            if let Err(error) = self.setup.users().lock(&user) {
                eprintln!("oxyos-accountd: {user}'s password cannot be turned off: {error}");
                let _ = store.put_local(&user, &before);
                return Err(LocalRefusal::Failed);
            }
        }
        Ok(local_view(&after))
    }

    fn check_password(self: &Arc<Self>, user: &str, password: Secret, local: Local) -> Checked {
        let enrollment = match self.setup.store().get_user(user) {
            Ok(Some(enrollment)) => enrollment,
            Ok(None) => return Checked::REJECTED,
            Err(error) => {
                eprintln!("oxyos-accountd: {user}'s account cannot be read: {error}");
                return Checked::REJECTED;
            }
        };
        if enrollment.state() == State::SignInAgain {
            return Checked::rejected(CheckReason::SignInAgain);
        }
        // Without OxyOS's client there is no platform sign-in to check with: PAM decides.
        if self.setup.platform().is_none() || !verifier_password(&password) {
            return match local {
                Local::Accepted => Checked::ACCEPTED,
                Local::Rejected => Checked::REJECTED,
            };
        }
        match (local, enrollment.password_changed) {
            (Local::Rejected, _) => match self.sign_in(&enrollment, &password, true).logged(user) {
                Online::Accepted | Online::PasswordRight => Checked::ACCEPTED,
                Online::Revoked => Checked::rejected(CheckReason::SignInAgain),
                // Changed elsewhere, and the authenticator is needed: set up again online.
                Online::SecondFactorRequired => Checked::rejected(CheckReason::PasswordChanged),
                Online::WrongPassword | Online::Offline | Online::Failed(_) => Checked::REJECTED,
            },
            (Local::Accepted, true) => {
                match self.sign_in(&enrollment, &password, false).logged(user) {
                    Online::Accepted | Online::PasswordRight => Checked::ACCEPTED,
                    // Offline the old password still opens it.
                    Online::Offline => Checked::ACCEPTED,
                    Online::Revoked => Checked::rejected(CheckReason::SignInAgain),
                    Online::WrongPassword | Online::Failed(_) | Online::SecondFactorRequired => {
                        Checked::rejected(CheckReason::PasswordChanged)
                    }
                }
            }
            (Local::Accepted, false) => {
                let platform = self.clone();
                let user = user.to_owned();
                let background = self.background.clone();
                *background.0.lock().unwrap_or_else(|p| p.into_inner()) += 1;
                let spawned = std::thread::Builder::new()
                    .name("oxyos-accountd-check".into())
                    .spawn(move || {
                        if let Ok(Some(enrollment)) = platform.setup.store().get_user(&user) {
                            if let Online::WrongPassword = platform
                                .sign_in(&enrollment, &password, false)
                                .logged(&user)
                            {
                                platform.flag_password_changed(&enrollment);
                            }
                        }
                        let (count, settled) = &*platform.background;
                        *count.lock().unwrap_or_else(|p| p.into_inner()) -= 1;
                        settled.notify_all();
                    });
                if spawned.is_err() {
                    *self.background.0.lock().unwrap_or_else(|p| p.into_inner()) -= 1;
                }
                Checked::ACCEPTED
            }
        }
    }

    /// Waits for the checks still asking Oxy in the background (the tests', and a stopping
    /// service's).
    pub fn settle(&self) {
        let (count, settled) = &*self.background;
        let mut count = count.lock().unwrap_or_else(|p| p.into_inner());
        while *count > 0 {
            count = settled.wait(count).unwrap_or_else(|p| p.into_inner());
        }
    }

    /// The platform's own sign-in with `password`, proving the account's device: Oxy rotates the
    /// platform credential, and the new one replaces the old in the account's file. With
    /// `set_verifier`, an accepted password also becomes the account's verifier (it was changed
    /// elsewhere). An accepted password clears the account's flag.
    fn sign_in(&self, enrollment: &Enrollment, password: &Secret, set_verifier: bool) -> Online {
        let Some(platform) = self.setup.platform() else {
            return Online::Failed("not configured".into());
        };
        if self.erasing() {
            return Online::Failed("the machine is being erased".into());
        }
        let calls = self.online_calls(enrollment.user.as_str());
        let _calls = calls.lock().unwrap_or_else(|p| p.into_inner());
        let api = self.setup.api();
        let Ok(Some(enrollment)) = self.setup.store().get(&enrollment.account) else {
            return Online::Failed("the account's file cannot be read".into());
        };
        let Some(credential) = enrollment.credential.as_ref() else {
            return Online::Revoked;
        };
        // The device's own token first: the credential is live (else the account was signed out
        // elsewhere), and the account's profile is read anew.
        let token = match api.mint_token(credential, &enrollment.account) {
            Ok(token) => token,
            Err(ApiError::Offline(_)) => {
                self.reached(&enrollment, false);
                return Online::Offline;
            }
            Err(ApiError::InvalidDeviceSecret | ApiError::AccountNotOnDevice) => {
                self.reached(&enrollment, true);
                self.revoked(&enrollment, credential);
                return Online::Revoked;
            }
            Err(error) => {
                self.reached(&enrollment, true);
                return Online::Failed(error.to_string());
            }
        };
        self.reached(&enrollment, true);
        let profile = match api.profile(&token) {
            Ok(profile) if profile.id == enrollment.account.as_str() => Some(profile),
            _ => None,
        };
        drop(token);
        if let Some(profile) = &profile {
            self.update_profile(&enrollment, credential, profile);
            // Its picture read anew when its avatar changed (or was never read): subscribers told.
            if self.setup.picture_user(&enrollment.user, profile) {
                self.changed(&enrollment);
            }
        }
        let username = profile
            .as_ref()
            .and_then(|profile| profile.username.clone())
            .or_else(|| enrollment.profile.username.clone());
        let Some(username) = username else {
            return Online::Failed("the account has no username".into());
        };
        let signed_in = match api.sign_in_password(&username, password, credential, Some(platform))
        {
            Ok(SignInStep::SignedIn(signed_in)) => signed_in,
            Ok(SignInStep::SecondFactor { .. }) => {
                if set_verifier {
                    return Online::SecondFactorRequired;
                }
                self.clear_flag(&enrollment, credential);
                return Online::PasswordRight;
            }
            Err(ApiError::InvalidCredentials) => return Online::WrongPassword,
            Err(ApiError::Offline(_)) => {
                self.reached(&enrollment, false);
                return Online::Offline;
            }
            Err(error) => return Online::Failed(error.to_string()),
        };
        let rotated = match signed_in.device_secret {
            Some(secret)
                if signed_in.user_id == enrollment.account.as_str()
                    && signed_in.device_id == credential.device_id =>
            {
                DeviceCredential {
                    device_id: signed_in.device_id,
                    device_secret: secret,
                }
            }
            _ => {
                eprintln!(
                    "oxyos-accountd: {}: Oxy's sign-in answered for another account or device, \
                     or issued no credential: kept the one it had",
                    enrollment.user
                );
                return Online::Failed("an answer outside the contract".into());
            }
        };
        // The rotated credential is pending until Oxy confirms it (ADR 0058 R8), and the one it
        // replaces works until then: unconfirmed, it expires by itself and the machine keeps the
        // one it has. Confirmed, it is the one the machine keeps.
        let rotated = match api.confirm_platform_credential(&rotated) {
            Ok(()) => Some(rotated),
            Err(error) => {
                eprintln!(
                    "oxyos-accountd: {}: the rotated credential could not be confirmed ({error}): \
                     kept the one it had",
                    enrollment.user
                );
                None
            }
        };
        if set_verifier {
            if let Err(error) = self.setup.users().set_verifier(&enrollment.user, password) {
                // The password is right, and the credential was rotated all the same: kept.
                eprintln!(
                    "oxyos-accountd: {}: the new password could not become the verifier: {error}",
                    enrollment.user
                );
                if let Some(rotated) = rotated {
                    self.rotate(&enrollment, credential, rotated);
                }
                return Online::Failed(error.to_string());
            }
        }
        if let Some(rotated) = rotated {
            self.rotate(&enrollment, credential, rotated);
        }
        Online::Accepted
    }

    /// Replaces `old` with `new` in the account's file, its flag cleared, if the file still holds
    /// `old`; else `new` is not the machine's and is ended online.
    fn rotate(&self, enrollment: &Enrollment, old: &DeviceCredential, new: DeviceCredential) {
        let written = self.rewrite(enrollment, old, |stored| {
            stored.credential = Some(copy(&new));
            stored.password_changed = false;
        });
        match written {
            Ok(true) => {
                // The old session ended with the rotation: its tokens are not handed out again.
                self.with_live(enrollment.user.as_str(), |live| live.tokens.clear());
                self.changed(enrollment);
            }
            Ok(false) => self.setup.end_online(&new, &enrollment.account),
            Err(error) => {
                eprintln!(
                    "oxyos-accountd: {}: the rotated credential could not be written: {error}; \
                     the account must be set up again",
                    enrollment.user
                );
                self.setup.end_online(&new, &enrollment.account);
            }
        }
    }

    fn clear_flag(&self, enrollment: &Enrollment, credential: &DeviceCredential) {
        if enrollment.password_changed {
            let _ = self.rewrite(enrollment, credential, |stored| {
                stored.password_changed = false
            });
        }
    }

    fn flag_password_changed(&self, enrollment: &Enrollment) {
        let Some(credential) = enrollment.credential.as_ref() else {
            return;
        };
        match self.rewrite(enrollment, credential, |stored| {
            stored.password_changed = true
        }) {
            Ok(true) => eprintln!(
                "oxyos-accountd: {}: Oxy refused the password the machine accepts: it was \
                 changed elsewhere, and is asked for online",
                enrollment.user
            ),
            Ok(false) => {}
            Err(error) => eprintln!("oxyos-accountd: {}: {error}", enrollment.user),
        }
    }

    fn update_profile(
        &self,
        enrollment: &Enrollment,
        credential: &DeviceCredential,
        profile: &Profile,
    ) {
        if &enrollment.profile == profile {
            return;
        }
        if let Ok(true) = self.rewrite(enrollment, credential, |stored| {
            stored.profile = profile.clone()
        }) {
            // A name changed on Oxy is the person's name here too: the user's full name, and
            // every snapshot's subscriber (the lock screen, the menu bar, Settings) told.
            if profile.name() != enrollment.profile.name() {
                let _machine = self.setup.lock_machine();
                self.setup.name_user(&enrollment.user, profile);
            }
            self.changed(enrollment);
        }
    }

    /// Rewrites the account's file with `change`, under the machine's lock, if it still holds
    /// `credential`: whether it did.
    fn rewrite(
        &self,
        enrollment: &Enrollment,
        credential: &DeviceCredential,
        change: impl FnOnce(&mut Enrollment),
    ) -> std::io::Result<bool> {
        let _machine = self.setup.lock_machine();
        let Some(mut stored) = self.setup.store().get(&enrollment.account)? else {
            return Ok(false);
        };
        if !stored
            .credential
            .as_ref()
            .is_some_and(|held| same(held, credential))
        {
            return Ok(false);
        }
        change(&mut stored);
        self.setup.store().put(&stored)?;
        Ok(true)
    }

    /// Oxy says `credential`'s device session is gone: the account is signed out on the machine,
    /// if its file still holds that credential (else a setup or a rotation has replaced it).
    fn revoked(&self, enrollment: &Enrollment, credential: &DeviceCredential) {
        let forgotten = self.rewrite(enrollment, credential, |stored| {
            stored.credential = None;
            stored.password_changed = false;
        });
        match forgotten {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => {
                eprintln!(
                    "oxyos-accountd: {}: signed out by Oxy, and its file cannot be changed: {error}",
                    enrollment.user
                );
                return;
            }
        }
        eprintln!(
            "oxyos-accountd: {}: Oxy ended its device session: signed out on this machine; it \
             signs in again online",
            enrollment.user
        );
        self.with_live(enrollment.user.as_str(), |live| live.tokens.clear());
        if let Err(error) = self.setup.users().lock(&enrollment.user) {
            eprintln!(
                "oxyos-accountd: {}: its verifier cannot be locked: {error}",
                enrollment.user
            );
        }
        self.switched(enrollment);
        if let Err(error) = self.setup.users().end_sessions(&enrollment.user) {
            eprintln!(
                "oxyos-accountd: {}: its sessions cannot be ended: {error}",
                enrollment.user
            );
        }
    }

    /// Removes `uid`'s account (decision 5), the part that must be done before the answer: its
    /// device ended online when Oxy can be reached, its verifier locked, its credential forgotten.
    /// [`Platform::finish_removal`] does the rest once the answer is sent.
    pub fn remove(&self, uid: u32) -> Result<Enrollment, RemoveError> {
        let enrollment = self
            .enrollment_of(uid)
            .map_err(|error| RemoveError::Failed(error.to_string()))?
            .ok_or(RemoveError::NotEnrolled)?;
        let calls = self.online_calls(enrollment.user.as_str());
        let _calls = calls.lock().unwrap_or_else(|p| p.into_inner());
        let enrollment = self
            .setup
            .store()
            .get(&enrollment.account)
            .map_err(|error| RemoveError::Failed(error.to_string()))?
            .ok_or(RemoveError::NotEnrolled)?;
        if let Some(keychains) = self.keychains() {
            keychains.lock(enrollment.uid);
        }
        if let Some(credential) = &enrollment.credential {
            self.setup.end_online(credential, &enrollment.account);
        }
        if let Err(error) = self.setup.users().lock(&enrollment.user) {
            eprintln!(
                "oxyos-accountd: {}: its verifier cannot be locked: {error}",
                enrollment.user
            );
        }
        {
            let _machine = self.setup.lock_machine();
            let forgotten = Enrollment {
                account: enrollment.account.clone(),
                user: enrollment.user.clone(),
                uid: enrollment.uid,
                profile: enrollment.profile.clone(),
                credential: None,
                password_changed: false,
            };
            self.setup
                .store()
                .put(&forgotten)
                .map_err(|error| RemoveError::Failed(error.to_string()))?;
        }
        self.with_live(enrollment.user.as_str(), |live| live.tokens.clear());
        self.switched(&enrollment);
        eprintln!(
            "oxyos-accountd: {}: removed from this machine; its sessions end",
            enrollment.user
        );
        Ok(enrollment)
    }

    /// Removing an account, the rest: its sessions ended, then its user and home removed (once its
    /// processes have gone), its file and picture forgotten. If the user cannot be removed, its
    /// file stays, signed out, so setting it up again finds its user.
    pub fn finish_removal(&self, enrollment: &Enrollment) {
        let user = &enrollment.user;
        if let Err(error) = self.setup.users().end_sessions(user) {
            eprintln!("oxyos-accountd: {user}: its sessions cannot be ended: {error}");
        }
        let deadline = std::time::Instant::now() + REMOVE_WAIT;
        let removed = loop {
            match self.setup.users().remove(user) {
                Ok(()) | Err(UsersError::Missing) => break Ok(()),
                Err(error) if std::time::Instant::now() >= deadline => break Err(error),
                // Its processes are still ending.
                Err(_) => std::thread::sleep(Duration::from_millis(500)),
            }
        };
        if let Err(error) = removed {
            eprintln!(
                "oxyos-accountd: {user}: the user and its home cannot be removed: {error}; it \
                 stays signed out"
            );
            return;
        }
        if let Some(keychains) = self.keychains() {
            keychains.forget(enrollment.uid, user.as_str());
        }
        let _machine = self.setup.lock_machine();
        if let Err(error) = self.setup.store().remove(user) {
            eprintln!("oxyos-accountd: {user}: its file cannot be removed: {error}");
        }
        if let Err(error) = self.setup.store().remove_pictures(user) {
            eprintln!("oxyos-accountd: {user}: its picture cannot be removed: {error}");
        }
        self.live
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(user.as_str());
    }

    /// Whether Oxy was reached by the last call made for `user`'s account, as its snapshot says.
    pub fn online(&self, user: &UserName) -> bool {
        self.with_live(user.as_str(), |live| live.online)
    }

    /// The machine's erase is armed (ADR 0054): from now on no token is minted and no password is
    /// checked online; only [`Platform::sign_out_for_erase`] asks Oxy anything. Recorded in the
    /// store, so the service started again before the machine restarts keeps to it.
    pub fn begin_erasing(&self) {
        self.erasing.store(true, Ordering::SeqCst);
        if let Err(error) = self.setup.store().record_fact(ERASE_ARMED) {
            eprintln!(
                "oxyos-accountd: that the erase is armed cannot be recorded: {error}; this \
                 service keeps to it until it stops"
            );
        }
    }

    pub fn erasing(&self) -> bool {
        self.erasing.load(Ordering::SeqCst)
    }

    /// Signs `user`'s account out of this device online, as removing it does (its own token
    /// minted, `POST /session/device/signout {all: true}`), under its online calls and with the
    /// credential its file holds now. Ok when Oxy ended it, or there was nothing left to end (no
    /// credential, a session already gone); else why not (offline, refused). The file is left as
    /// it is: the erase takes it with the disk.
    pub fn sign_out_for_erase(&self, user: &UserName) -> Result<(), String> {
        let calls = self.online_calls(user.as_str());
        let _calls = calls.lock().unwrap_or_else(|p| p.into_inner());
        let enrollment = self
            .setup
            .store()
            .get_user(user.as_str())
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "no longer enrolled".to_owned())?;
        let Some(credential) = &enrollment.credential else {
            return Ok(());
        };
        self.setup
            .sign_out_online(credential, &enrollment.account)
            .map_err(|error| error.to_string())
    }

    /// The account `user` names, if it is enrolled.
    pub fn account_of(&self, user: &str) -> Option<AccountId> {
        self.setup
            .store()
            .get_user(user)
            .ok()
            .flatten()
            .map(|enrollment| enrollment.account)
    }
}

/// An account's local sign-in, as Settings and the lock screen are told it (never a verifier).
fn local_view(local: &crate::pin::LocalSignIn) -> LocalView {
    LocalView {
        pin: local.pin_state(),
        password: local.password,
    }
}
