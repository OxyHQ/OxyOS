//! Setting up an account on the machine ([ADR 0020](../../docs/adr/0020-oxy-account-login.mdx),
//! decision 2 and step 1), from the device credential the SDK's sign-in obtained and the
//! account's password, given once more.
//!
//! Online, before anything changes on the machine:
//!
//! 1. the device credential mints a token pinned to the account: the credential is live and the
//!    account is signed in on that device;
//! 2. the token names the account (`/users/me`): it is the one asked for, and its username is
//!    what the password signs in with;
//! 3. Oxy checks the password, the sign-in proving the same device and naming OxyOS's public
//!    client (`platformClient`), and asks for the authenticator's code if the account has one
//!    ([`Progress::SecondFactor`]);
//! 4. the sign-in must be that account's, on that device, and give a holder secret: the
//!    **platform credential**, which only this service ever received. Oxy retires the credential
//!    handed over (the one the setup client saw) in the same transaction, so the platform
//!    credential is the one the machine keeps.
//!
//! Without OxyOS's client id configured, nothing is asked of Oxy: [`SetupError::NotConfigured`].
//!
//! Then, on the machine, one setup at a time: the account's user (made, or the enrolled one
//! found), its file in the store, the account made the machine's administrator if it is the first
//! (as a Mac's first account is, and once: [ADR 0054](../../docs/adr/0054-recovery-and-erasing.mdx),
//! the store's `ADMINISTRATOR_MADE`), and last the verifier. A failure
//! undoes what this setup did: a new user is removed with its home (and so leaves the
//! administrators' group), an enrolled account's file is put back as it was. The
//! verifier is the last step, so a user whose setup failed was never one a password opens. A
//! platform credential a failed setup was issued is ended online, as is the one an account set up
//! again replaces (each is a device of its own): best effort, said if it fails.
//!
//! Without the network the first step fails, and nothing is done: a first setup never succeeds
//! offline.
//!
//! **The local sign-in** (`crate::pin`): the password becomes the verifier, as above, and a PIN
//! given with it (4 to 16 digits, never the password itself) becomes the PIN's verifier, in the
//! account's `.local` file, written with the account's file and undone with it. An account set up
//! again without a PIN keeps the one it had, its wrong attempts forgotten: Oxy was just asked.
//!
//! **An account without a password** cannot be set up yet: the platform credential is issued only
//! by the platform's own password sign-in (Oxy ADR 0058, 2.1), and Oxy has no other way for a
//! platform to prove the setup client's credential and be issued its own. Such a setup is refused
//! [`SetupError::PasswordRequired`] before anything is asked of Oxy or changed here; the PIN alone
//! will set it up once Oxy has that way.

use crate::api::{
    ApiError, DeviceCredential, OxyApi, PlatformClient, Profile, SignInStep, SignedIn,
};
use crate::id::{AccountId, InvalidAccountId, UserName};
use crate::pin::{usable_pin, LocalSignIn, PinRecord};
use crate::secret::Secret;
use crate::store::{Enrollment, Store, ADMINISTRATOR_MADE};
use crate::users::{verifier_password, Users, UsersError};
use std::fmt;
use std::sync::Mutex;

/// What the setup client hands over: the SDK's credential, the account it signed in, the
/// account's password (none: the account has none) and the PIN chosen for this computer, if any.
#[derive(Debug)]
pub struct Request {
    pub credential: DeviceCredential,
    pub account: String,
    pub password: Option<Secret>,
    pub pin: Option<Secret>,
}

/// An enrolled account, as the setup client is told of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enrolled {
    pub account: AccountId,
    pub user: UserName,
    pub uid: u32,
    pub profile: Profile,
}

/// A setup waiting for the authenticator's code. Its secrets are overwritten when it is dropped.
#[derive(Debug)]
pub struct Pending {
    account: AccountId,
    credential: DeviceCredential,
    password: Secret,
    /// The PIN's verifier, made before Oxy was asked.
    pin: Option<String>,
    profile: Profile,
    challenge: Secret,
    platform: PlatformClient,
}

#[derive(Debug)]
pub enum Progress {
    Enrolled(Enrolled),
    /// The account has an authenticator: [`Setup::second_factor`] with its code.
    SecondFactor(Pending),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupError {
    /// OxyOS's public client id is not configured (`OXYOS_OXY_CLIENT_ID`): nothing is set up.
    NotConfigured,
    /// Not an Oxy account id this machine can name.
    InvalidAccount,
    /// Empty, longer than PAM takes, or holding a NUL or a line break.
    UnusablePassword,
    /// Not 4 to 16 digits, or the account's password itself.
    UnusablePin,
    /// The account has no password, and the platform credential is issued only by the platform's
    /// password sign-in: Oxy has no way yet to set up an account with a PIN alone.
    PasswordRequired,
    /// Oxy cannot be reached: setup needs the network.
    Offline,
    /// Oxy does not know the device credential.
    CredentialRefused,
    /// The account is not signed in with that credential.
    AccountNotOnDevice,
    /// The password is not the account's.
    WrongPassword,
    /// The authenticator's code is not right; another may be tried while the challenge lasts.
    SecondFactorInvalid,
    /// Too many attempts at Oxy.
    Locked { retry_after: Option<u64> },
    /// What Oxy answered is another account's or another device's.
    AccountMismatch,
    /// The account has no username for its password to sign in with.
    NoUsername,
    /// Oxy refused for another reason, or answered outside its contract.
    Oxy(String),
    /// The machine holds a user of the account's name that is not the enrolled account's, or an
    /// enrolled account whose user is gone. Nothing is changed: an administrator decides.
    Conflict(String),
    /// The machine could not be changed; what this setup did was undone.
    Failed(String),
    /// The machine could not be changed, and undoing it failed too.
    RollbackFailed { error: String, rollback: String },
}

impl fmt::Display for SetupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SetupError::NotConfigured => f.write_str("OxyOS's client id is not configured"),
            SetupError::InvalidAccount => {
                f.write_str("not an Oxy account id this machine can name")
            }
            SetupError::UnusablePassword => {
                f.write_str("the password cannot be checked by the machine")
            }
            SetupError::UnusablePin => {
                f.write_str("a PIN is 4 to 16 digits, and not the account's password")
            }
            SetupError::PasswordRequired => f.write_str(
                "an account without a password cannot be set up yet: Oxy issues the platform \
                 credential only to the platform's password sign-in",
            ),
            SetupError::Offline => f.write_str("setting up an account needs the network"),
            SetupError::CredentialRefused => f.write_str("Oxy does not know the device credential"),
            SetupError::AccountNotOnDevice => {
                f.write_str("the account is not signed in with the credential")
            }
            SetupError::WrongPassword => f.write_str("the password is not right"),
            SetupError::SecondFactorInvalid => f.write_str("the authenticator's code is not right"),
            SetupError::Locked {
                retry_after: Some(seconds),
            } => {
                write!(f, "too many attempts: try again in {seconds} s")
            }
            SetupError::Locked { retry_after: None } => f.write_str("too many attempts"),
            SetupError::AccountMismatch => f.write_str("Oxy signed in another account or device"),
            SetupError::NoUsername => f.write_str("the account has no username to sign in with"),
            SetupError::Oxy(why) => write!(f, "Oxy: {why}"),
            SetupError::Conflict(why) => write!(f, "the machine's users disagree: {why}"),
            SetupError::Failed(why) => write!(f, "the account could not be set up (undone): {why}"),
            SetupError::RollbackFailed { error, rollback } => {
                write!(
                    f,
                    "the account could not be set up: {error}; undoing it failed: {rollback}"
                )
            }
        }
    }
}

impl std::error::Error for SetupError {}

impl From<ApiError> for SetupError {
    fn from(error: ApiError) -> Self {
        match error {
            ApiError::Offline(_) => SetupError::Offline,
            ApiError::InvalidDeviceSecret => SetupError::CredentialRefused,
            ApiError::AccountNotOnDevice => SetupError::AccountNotOnDevice,
            ApiError::InvalidCredentials => SetupError::WrongPassword,
            ApiError::SecondFactorInvalid => SetupError::SecondFactorInvalid,
            ApiError::Locked { retry_after } => SetupError::Locked { retry_after },
            other => SetupError::Oxy(other.to_string()),
        }
    }
}

/// What the machine holds for an account before it is set up.
enum Local {
    /// Neither a user nor an enrollment.
    New,
    /// Enrolled, its user as it was enrolled.
    Enrolled(Box<Enrollment>),
}

pub struct Setup<A, U> {
    api: A,
    users: U,
    store: Store,
    /// OxyOS's public client, which the platform's sign-ins name; none configured, nothing is
    /// set up.
    platform: Option<PlatformClient>,
    /// One change to the machine's accounts (a setup, a credential rotated or forgotten) at a
    /// time.
    machine: Mutex<()>,
}

impl<A: OxyApi, U: Users> Setup<A, U> {
    pub fn new(api: A, users: U, store: Store) -> Self {
        Self {
            api,
            users,
            store,
            platform: None,
            machine: Mutex::new(()),
        }
    }

    /// Setups name OxyOS's public client `client`.
    pub fn with_platform(mut self, client: PlatformClient) -> Self {
        self.platform = Some(client);
        self
    }

    pub fn platform(&self) -> Option<&PlatformClient> {
        self.platform.as_ref()
    }

    pub fn api(&self) -> &A {
        &self.api
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn users(&self) -> &U {
        &self.users
    }

    /// Sets the account up, or asks for its authenticator's code.
    pub fn enroll(&self, request: Request) -> Result<Progress, SetupError> {
        let platform = self.platform.clone().ok_or(SetupError::NotConfigured)?;
        let account = AccountId::parse(&request.account)
            .map_err(|InvalidAccountId| SetupError::InvalidAccount)?;
        let Some(password) = request.password else {
            return Err(SetupError::PasswordRequired);
        };
        if !verifier_password(&password) {
            return Err(SetupError::UnusablePassword);
        }
        let pin = match &request.pin {
            None => None,
            Some(pin) if !usable_pin(pin) || pin.expose() == password.expose() => {
                return Err(SetupError::UnusablePin)
            }
            Some(pin) => Some(pin_verifier(pin)?),
        };
        // A machine that could not take the account is found out before Oxy is asked.
        {
            let _machine = self.lock();
            self.local(&account)?;
        }
        let profile = {
            let token = self.api.mint_token(&request.credential, &account)?;
            self.api.profile(&token)?
        };
        if profile.id != account.as_str() {
            return Err(SetupError::AccountMismatch);
        }
        let username = profile.username.clone().ok_or(SetupError::NoUsername)?;
        match self.api.sign_in_password(
            &username,
            &password,
            &request.credential,
            Some(&platform),
        )? {
            SignInStep::SignedIn(signed_in) => self
                .finish(
                    &account,
                    &request.credential,
                    &password,
                    pin.as_deref(),
                    profile,
                    signed_in,
                )
                .map(Progress::Enrolled),
            SignInStep::SecondFactor { challenge } => Ok(Progress::SecondFactor(Pending {
                account,
                credential: request.credential,
                password,
                pin,
                profile,
                challenge,
                platform,
            })),
        }
    }

    /// The authenticator's code for a setup that asked for it. A wrong code leaves `pending` to
    /// try another.
    pub fn second_factor(&self, pending: &Pending, code: &Secret) -> Result<Enrolled, SetupError> {
        let signed_in = self.api.second_factor(
            &pending.challenge,
            code,
            &pending.credential,
            Some(&pending.platform),
        )?;
        self.finish(
            &pending.account,
            &pending.credential,
            &pending.password,
            pending.pin.as_deref(),
            pending.profile.clone(),
            signed_in,
        )
    }

    /// The sign-in done: the platform credential it issued kept, or ended online if the machine
    /// could not take it; the credential an account set up again had, ended online.
    fn finish(
        &self,
        account: &AccountId,
        handed_over: &DeviceCredential,
        password: &Secret,
        pin: Option<&str>,
        profile: Profile,
        signed_in: SignedIn,
    ) -> Result<Enrolled, SetupError> {
        if signed_in.user_id != account.as_str() || signed_in.device_id != handed_over.device_id {
            // Not this device's sign-in: whatever it issued is not the machine's to end.
            return Err(SetupError::AccountMismatch);
        }
        let Some(secret) = signed_in.device_secret else {
            return Err(SetupError::Oxy(
                "the sign-in issued no platform credential".into(),
            ));
        };
        let credential = DeviceCredential {
            device_id: signed_in.device_id,
            device_secret: secret,
        };
        match self.commit(account, &credential, password, pin, profile) {
            Ok((enrolled, replaced)) => {
                if let Some(replaced) = replaced.filter(|old| old.device_id != credential.device_id)
                {
                    self.end_online(&replaced, account);
                }
                Ok(enrolled)
            }
            Err(error) => {
                self.end_online(&credential, account);
                Err(error)
            }
        }
    }

    /// Ends `credential`'s device session online (R4): its own token minted, then signed out.
    /// Best effort: the device would otherwise stay signed in at Oxy with no machine holding it.
    pub fn end_online(&self, credential: &DeviceCredential, account: &AccountId) {
        if let Err(error) = self.sign_out_online(credential, account) {
            eprintln!(
                "oxyos-accountd: a device of {account} could not be signed out at Oxy: {error}"
            );
        }
    }

    /// Signs `credential`'s device session out at Oxy (R4): its own token minted, then
    /// `POST /session/device/signout {all: true}`. A session already gone is signed out.
    pub fn sign_out_online(
        &self,
        credential: &DeviceCredential,
        account: &AccountId,
    ) -> Result<(), ApiError> {
        let ended = self
            .api
            .mint_token(credential, account)
            .and_then(|token| self.api.sign_out_device(&token));
        match ended {
            // Already gone: nothing to end.
            Ok(()) | Err(ApiError::InvalidDeviceSecret | ApiError::AccountNotOnDevice) => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Holds the machine's accounts unchanged while the caller reads and writes them.
    pub fn lock_machine(&self) -> std::sync::MutexGuard<'_, ()> {
        self.lock()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        // A setup that panicked holding it left the machine as its rollback did; go on.
        self.machine
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Whether an enrolled account is one of the machine's administrators. Called under the
    /// machine's lock.
    fn administrator_enrolled(&self) -> Result<bool, String> {
        for enrollment in self
            .store
            .enrollments()
            .map_err(|error| error.to_string())?
        {
            if self
                .users
                .is_administrator(&enrollment.user)
                .map_err(|error| error.to_string())?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Makes `user`, whose file was just written, the machine's first administrator, if it has
    /// had none: whether it did. A machine whose administrator exists has one made already (it is
    /// recorded); one whose administrator was made and then removed gets none from a setup. Called
    /// under the machine's lock, before the verifier is set.
    fn administer(&self, user: &UserName) -> Result<bool, String> {
        if self.store.fact(ADMINISTRATOR_MADE) {
            return Ok(false);
        }
        if self.administrator_enrolled()? {
            self.store
                .record_fact(ADMINISTRATOR_MADE)
                .map_err(|error| error.to_string())?;
            return Ok(false);
        }
        self.users
            .make_administrator(user)
            .map_err(|error| format!("{user} cannot be made an administrator: {error}"))?;
        Ok(true)
    }

    /// Records that `user`, whose setup just ended well, was made the machine's first
    /// administrator. If it cannot be, the next start records it (an administrator is enrolled).
    fn administered(&self, user: &UserName) {
        eprintln!("oxyos-accountd: {user} is this machine's administrator");
        if let Err(error) = self.store.record_fact(ADMINISTRATOR_MADE) {
            eprintln!(
                "oxyos-accountd: that the machine has its administrator cannot be recorded: {error}"
            );
        }
    }

    /// Makes the enrolled account with the lowest uid the machine's administrator, once: on a
    /// machine set up before administrators existed (accounts enrolled, none an administrator,
    /// none ever made one). Never again after that, so removing an administrator never makes
    /// another account one by itself (ADR 0054). Its user, if one was made.
    pub fn ensure_administrator(&self) -> Result<Option<UserName>, String> {
        let _machine = self.lock();
        if self.store.fact(ADMINISTRATOR_MADE) {
            return Ok(None);
        }
        let record = || {
            self.store
                .record_fact(ADMINISTRATOR_MADE)
                .map_err(|error| error.to_string())
        };
        if self.administrator_enrolled()? {
            record()?;
            return Ok(None);
        }
        let enrolled = self
            .store
            .enrollments()
            .map_err(|error| error.to_string())?;
        let Some(first) = enrolled.iter().min_by_key(|enrollment| enrollment.uid) else {
            return Ok(None);
        };
        self.users
            .make_administrator(&first.user)
            .map_err(|error| format!("{} cannot be made an administrator: {error}", first.user))?;
        record()?;
        Ok(Some(first.user.clone()))
    }

    fn local(&self, account: &AccountId) -> Result<Local, SetupError> {
        let user = account.user_name();
        let enrollment = self
            .store
            .get(account)
            .map_err(|error| SetupError::Failed(error.to_string()))?;
        let found = self
            .users
            .lookup(&user)
            .map_err(|error| SetupError::Failed(error.to_string()))?;
        match (enrollment, found) {
            (None, None) => Ok(Local::New),
            (Some(enrollment), Some(found)) if found.uid == enrollment.uid => {
                Ok(Local::Enrolled(Box::new(enrollment)))
            }
            (Some(enrollment), Some(found)) => Err(SetupError::Conflict(format!(
                "{user} is uid {}, enrolled as uid {}",
                found.uid, enrollment.uid
            ))),
            (Some(_), None) => Err(SetupError::Conflict(format!(
                "{user} is enrolled but has no user"
            ))),
            (None, Some(_)) => Err(SetupError::Conflict(format!(
                "a user {user} exists that is not an enrolled account"
            ))),
        }
    }

    /// The machine takes the account: its user, its file with `credential`, its verifier, its
    /// local sign-in (`pin`, a PIN's verifier). The credential an enrolled account had before, if
    /// any, is given back.
    fn commit(
        &self,
        account: &AccountId,
        credential: &DeviceCredential,
        password: &Secret,
        pin: Option<&str>,
        profile: Profile,
    ) -> Result<(Enrolled, Option<DeviceCredential>), SetupError> {
        let _machine = self.lock();
        let user = account.user_name();
        // The password signs in again; the PIN given replaces the one it had, and one kept has its
        // wrong attempts forgotten (Oxy was just asked).
        let previous_local = self.store.local(&user).ok();
        let local = LocalSignIn {
            pin: match pin {
                Some(verifier) => Some(PinRecord {
                    verifier: verifier.to_owned(),
                    failures: 0,
                }),
                None => previous_local
                    .as_ref()
                    .and_then(|local| local.pin.clone())
                    .map(|record| PinRecord {
                        failures: 0,
                        ..record
                    }),
            },
            password: true,
        };
        let enrollment = |uid| Enrollment {
            account: account.clone(),
            user: user.clone(),
            uid,
            profile: profile.clone(),
            credential: Some(DeviceCredential {
                device_id: credential.device_id.clone(),
                device_secret: Secret::new(credential.device_secret.expose()),
            }),
            password_changed: false,
        };
        let mut replaced = None;
        let uid = match self.local(account)? {
            Local::New => {
                let made = match self.users.create(&user) {
                    Ok(made) => made,
                    Err(UsersError::Exists) => {
                        return Err(SetupError::Conflict(format!(
                            "a user {user} exists in another database"
                        )))
                    }
                    // No user had the name a moment ago, under the lock: whatever useradd left
                    // of it is this setup's to remove.
                    Err(error) => {
                        let error = error.to_string();
                        return Err(match self.users.remove(&user) {
                            Ok(()) | Err(UsersError::Missing) => SetupError::Failed(error),
                            Err(rollback) => SetupError::RollbackFailed {
                                error,
                                rollback: rollback.to_string(),
                            },
                        });
                    }
                };
                let undo = || {
                    let removed = self.users.remove(&user).map_err(|e| e.to_string());
                    let forgotten = self.store.remove(&user).map_err(|e| e.to_string());
                    removed.and(forgotten)
                };
                let done = self
                    .store
                    .put(&enrollment(made.uid))
                    .and_then(|()| self.store.put_local(&user, &local))
                    .map_err(|error| error.to_string())
                    .and_then(|()| self.administer(&user))
                    .and_then(|administrator| {
                        self.users
                            .set_verifier(&user, password)
                            .map_err(|error| error.to_string())
                            .map(|()| administrator)
                    });
                match done {
                    Ok(true) => self.administered(&user),
                    Ok(false) => {}
                    Err(error) => {
                        return Err(match undo() {
                            Ok(()) => SetupError::Failed(error),
                            Err(rollback) => SetupError::RollbackFailed { error, rollback },
                        })
                    }
                }
                made.uid
            }
            Local::Enrolled(previous) => {
                let uid = previous.uid;
                self.store
                    .put(&enrollment(uid))
                    .map_err(|error| SetupError::Failed(error.to_string()))?;
                let done = self
                    .store
                    .put_local(&user, &local)
                    .map_err(|error| error.to_string())
                    .and_then(|()| self.administer(&user))
                    .and_then(|administrator| {
                        self.users
                            .set_verifier(&user, password)
                            .map_err(|error| error.to_string())
                            .map(|()| administrator)
                    });
                if let Ok(true) = done {
                    self.administered(&user);
                }
                if let Err(error) = done {
                    let local_back = match &previous_local {
                        Some(local) => self.store.put_local(&user, local),
                        None => Ok(()),
                    };
                    return Err(match self.store.put(&previous).and(local_back) {
                        Ok(()) => SetupError::Failed(error),
                        Err(rollback) => SetupError::RollbackFailed {
                            error,
                            rollback: rollback.to_string(),
                        },
                    });
                }
                replaced = previous.credential;
                uid
            }
        };
        Ok((
            Enrolled {
                account: account.clone(),
                user,
                uid,
                profile,
            },
            replaced,
        ))
    }
}

/// A PIN's verifier, made off the machine's lock (Argon2id takes a moment).
#[cfg(feature = "local")]
fn pin_verifier(pin: &Secret) -> Result<String, SetupError> {
    crate::pin::hash_pin(pin).map_err(|error| match error {
        crate::pin::VerifierError::Unusable => SetupError::UnusablePin,
        other => SetupError::Failed(other.to_string()),
    })
}

/// Without the service's verifiers, no PIN can be set up.
#[cfg(not(feature = "local"))]
fn pin_verifier(_pin: &Secret) -> Result<String, SetupError> {
    Err(SetupError::Failed(
        "this build makes no PIN verifiers".into(),
    ))
}
