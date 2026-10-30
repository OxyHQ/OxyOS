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
//! 3. Oxy checks the password, the sign-in proving the same device, and asks for the
//!    authenticator's code if the account has one ([`Progress::SecondFactor`]);
//! 4. the sign-in must be that account's, on that device.
//!
//! Then, on the machine, one setup at a time: the account's user (made, or the enrolled one
//! found), its file in the store, and last the verifier. A failure undoes what this setup did: a
//! new user is removed with its home, an enrolled account's file is put back as it was. The
//! verifier is the last step, so a user whose setup failed was never one a password opens.
//!
//! Without the network the first step fails, and nothing is done: a first setup never succeeds
//! offline.

use crate::api::{ApiError, DeviceCredential, OxyApi, Profile, SignInStep, SignedIn};
use crate::id::{AccountId, InvalidAccountId, UserName};
use crate::secret::Secret;
use crate::store::{Enrollment, Store};
use crate::users::{verifier_password, Users, UsersError};
use std::fmt;
use std::sync::Mutex;

/// What the setup client hands over: the SDK's credential, the account it signed in, and the
/// account's password.
#[derive(Debug)]
pub struct Request {
    pub credential: DeviceCredential,
    pub account: String,
    pub password: Secret,
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
    profile: Profile,
    challenge: Secret,
}

#[derive(Debug)]
pub enum Progress {
    Enrolled(Enrolled),
    /// The account has an authenticator: [`Setup::second_factor`] with its code.
    SecondFactor(Pending),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupError {
    /// Not an Oxy account id this machine can name.
    InvalidAccount,
    /// Empty, longer than PAM takes, or holding a NUL or a line break.
    UnusablePassword,
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
            SetupError::InvalidAccount => {
                f.write_str("not an Oxy account id this machine can name")
            }
            SetupError::UnusablePassword => {
                f.write_str("the password cannot be checked by the machine")
            }
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
    /// One setup changes the machine at a time.
    machine: Mutex<()>,
}

impl<A: OxyApi, U: Users> Setup<A, U> {
    pub fn new(api: A, users: U, store: Store) -> Self {
        Self {
            api,
            users,
            store,
            machine: Mutex::new(()),
        }
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
        let account = AccountId::parse(&request.account)
            .map_err(|InvalidAccountId| SetupError::InvalidAccount)?;
        if !verifier_password(&request.password) {
            return Err(SetupError::UnusablePassword);
        }
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
        match self
            .api
            .sign_in_password(&username, &request.password, &request.credential)?
        {
            SignInStep::SignedIn(signed_in) => self
                .commit(
                    &account,
                    &request.credential,
                    &request.password,
                    profile,
                    &signed_in,
                )
                .map(Progress::Enrolled),
            SignInStep::SecondFactor { challenge } => Ok(Progress::SecondFactor(Pending {
                account,
                credential: request.credential,
                password: request.password,
                profile,
                challenge,
            })),
        }
    }

    /// The authenticator's code for a setup that asked for it. A wrong code leaves `pending` to
    /// try another.
    pub fn second_factor(&self, pending: &Pending, code: &Secret) -> Result<Enrolled, SetupError> {
        let signed_in = self
            .api
            .second_factor(&pending.challenge, code, &pending.credential)?;
        self.commit(
            &pending.account,
            &pending.credential,
            &pending.password,
            pending.profile.clone(),
            &signed_in,
        )
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        // A setup that panicked holding it left the machine as its rollback did; go on.
        self.machine
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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

    fn commit(
        &self,
        account: &AccountId,
        credential: &DeviceCredential,
        password: &Secret,
        profile: Profile,
        signed_in: &SignedIn,
    ) -> Result<Enrolled, SetupError> {
        if signed_in.user_id != account.as_str() || signed_in.device_id != credential.device_id {
            return Err(SetupError::AccountMismatch);
        }
        let _machine = self.lock();
        let user = account.user_name();
        let enrollment = |uid| Enrollment {
            account: account.clone(),
            user: user.clone(),
            uid,
            profile: profile.clone(),
            credential: DeviceCredential {
                device_id: credential.device_id.clone(),
                device_secret: Secret::new(credential.device_secret.expose()),
            },
        };
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
                    .map_err(|error| error.to_string())
                    .and_then(|()| {
                        self.users
                            .set_verifier(&user, password)
                            .map_err(|error| error.to_string())
                    });
                if let Err(error) = done {
                    return Err(match undo() {
                        Ok(()) => SetupError::Failed(error),
                        Err(rollback) => SetupError::RollbackFailed { error, rollback },
                    });
                }
                made.uid
            }
            Local::Enrolled(previous) => {
                let uid = previous.uid;
                self.store
                    .put(&enrollment(uid))
                    .map_err(|error| SetupError::Failed(error.to_string()))?;
                if let Err(error) = self.users.set_verifier(&user, password) {
                    let error = error.to_string();
                    return Err(match self.store.put(&previous) {
                        Ok(()) => SetupError::Failed(error),
                        Err(rollback) => SetupError::RollbackFailed {
                            error,
                            rollback: rollback.to_string(),
                        },
                    });
                }
                uid
            }
        };
        Ok(Enrolled {
            account: account.clone(),
            user,
            uid,
            profile,
        })
    }
}
