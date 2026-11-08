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
//! verifier is the last step, so a user whose setup failed was never one a password opens. The
//! platform credential an account set up again replaces is ended online (each is a device of its
//! own), and so is one a setup was issued and never finished with (below): best effort, said if it
//! fails.
//!
//! Without the network the first step fails, and nothing is done: a first setup never succeeds
//! offline.
//!
//! **The local sign-in** (`crate::pin`): the password becomes the verifier, as above, and a PIN
//! given with it (4 to 16 digits, never the password itself) becomes the PIN's verifier, in the
//! account's `.local` file, written with the account's file and undone with it. An account set up
//! again without a PIN keeps the one it had, its wrong attempts forgotten: Oxy was just asked.
//!
//! **A sign-in by the QR** (a Commons approval, no password involved; Oxy ADR 0058's device-proof
//! route): the setup carries a PIN and no password. Oxy redeems the setup client's credential, the
//! one the QR's claim issued it, once and within ten minutes of the claim, for the platform
//! credential (`POST /session/device/platform`), retiring it in the same transaction. The PIN is
//! the account's local credential: no password is asked, offered or made, the user's verifier stays
//! locked (an enrolled account set up again keeps the one it had), and its password does not sign
//! in here until Settings turns it on. A setup with neither a password nor a PIN is refused
//! [`SetupError::PinRequired`] before Oxy is asked.

use crate::api::{
    ApiError, DeviceCredential, OxyApi, PlatformClient, Profile, RedemptionKey, SignInStep,
    SignedIn,
};
use crate::id::{AccountId, InvalidAccountId, UserName};
use crate::pin::{usable_pin, LocalSignIn, PinRecord};
use crate::secret::Secret;
use crate::store::{Enrollment, Store, ADMINISTRATOR_MADE};
use crate::users::{verifier_password, UnixUser, Users, UsersError};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

/// What the setup client hands over: the SDK's credential, the account it signed in, the
/// account's password (none: the account has none), the PIN chosen for this computer, if any, and
/// the appearance a new account's home starts with, if one was chosen.
#[derive(Debug)]
pub struct Request {
    pub credential: DeviceCredential,
    pub account: String,
    pub password: Option<Secret>,
    pub pin: Option<Secret>,
    pub appearance: Option<Appearance>,
}

/// The appearance the setup assistant asks for, as the preferences name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Appearance {
    Light,
    Dark,
    Auto,
}

impl Appearance {
    pub fn name(self) -> &'static str {
        match self {
            Appearance::Light => "light",
            Appearance::Dark => "dark",
            Appearance::Auto => "auto",
        }
    }
}

/// A setup's step, reported as it starts (see the module's doc for what each is).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    Securing,
    Verifying,
    Preparing,
    SigningIn,
    Finishing,
}

/// How long a setup waits before asking Oxy's redemption again after a failure on the way: the
/// redemption is asked once and then once after each of these.
pub const RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(5)];

/// An enrolled account, as the setup client is told of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enrolled {
    pub account: AccountId,
    pub user: UserName,
    pub uid: u32,
    pub profile: Profile,
}

/// One setup across its conversation's turns (`crate::ipc`): what was handed over, checked; the
/// key its redemption is made with; the authenticator's challenge while its code is asked for;
/// and the platform credential a sign-in issued that the machine could not take yet. In memory
/// alone, never logged; its secrets are overwritten when it is dropped. A platform credential it
/// still holds when its conversation ends without success is ended online ([`Setup::abandon`]).
#[derive(Debug)]
pub struct Attempt {
    account: AccountId,
    /// The credential handed over: the setup client's, the QR's proof.
    credential: DeviceCredential,
    password: Option<Secret>,
    /// The PIN given, until its verifier is made.
    pin_given: Option<Secret>,
    /// The PIN's verifier, made before Oxy is asked, once.
    pin: Option<String>,
    appearance: Option<Appearance>,
    key: RedemptionKey,
    /// The account as Oxy named it, once it has: checked once per setup.
    profile: Option<Profile>,
    /// The authenticator's challenge: its code is asked for.
    challenge: Option<Secret>,
    /// The platform credential a sign-in issued, which the machine has not taken yet.
    held: Option<DeviceCredential>,
}

impl Attempt {
    /// Whether the authenticator's code is asked for.
    pub fn second_factor_asked(&self) -> bool {
        self.challenge.is_some()
    }

    /// Whether it holds a platform credential the machine has not taken yet.
    pub fn holds_credential(&self) -> bool {
        self.held.is_some()
    }
}

/// A setup waiting for the authenticator's code: [`Setup::second_factor`] with it.
pub type Pending = Attempt;

#[derive(Debug)]
pub enum Progress {
    Enrolled(Enrolled),
    /// The account has an authenticator: [`Setup::second_factor`] with its code.
    SecondFactor(Pending),
}

/// Where a setup's turn left it.
#[derive(Debug)]
pub enum Step {
    Enrolled(Enrolled),
    /// The account has an authenticator: [`Setup::second_factor_of`] with its code.
    SecondFactor,
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
    /// No password and no PIN: an account set up by its QR needs a PIN, its one local credential.
    PinRequired,
    /// Oxy cannot be reached, or failed for a moment (a timeout, a 5xx): setup needs the network.
    /// Nothing was spent that cannot be asked for again: the same setup may try again.
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
    /// The machine could not be changed; what this setup did was undone. A sign-in it made is kept
    /// by its [`Attempt`]: the same setup may try again.
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
            SetupError::PinRequired => {
                f.write_str("an account set up without its password needs a PIN")
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
            // Oxy failing for a moment is no answer about the account: as unreachable.
            ApiError::Refused {
                status: 500..=599, ..
            } => SetupError::Offline,
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

/// The machine made ready for an account before its credential is taken: a new account's user
/// made by this setup (its home with it), or an enrolled account's user found as enrolled.
enum Prepared {
    Made(UnixUser),
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
    /// The pauses before a redemption that failed on its way is asked again.
    retry_delays: Vec<Duration>,
}

impl<A: OxyApi, U: Users> Setup<A, U> {
    pub fn new(api: A, users: U, store: Store) -> Self {
        Self {
            api,
            users,
            store,
            platform: None,
            machine: Mutex::new(()),
            retry_delays: RETRY_DELAYS.to_vec(),
        }
    }

    /// A redemption that failed on its way is asked again after each of `delays` (the tests'
    /// own, without waiting).
    pub fn with_retry_delays(mut self, delays: Vec<Duration>) -> Self {
        self.retry_delays = delays;
        self
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

    /// Sets the account up, or asks for its authenticator's code: a setup of its own, which a
    /// failure ends (a platform credential it was issued ended online with it). The setup
    /// client's conversation, which may try again, uses [`Setup::start`] and [`Setup::advance`].
    pub fn enroll(&self, request: Request) -> Result<Progress, SetupError> {
        let mut attempt = self.start(request)?;
        match self.advance(&mut attempt, &mut |_| {}) {
            Ok(Step::Enrolled(enrolled)) => Ok(Progress::Enrolled(enrolled)),
            Ok(Step::SecondFactor) => Ok(Progress::SecondFactor(attempt)),
            Err(error) => {
                self.abandon(&mut attempt);
                Err(error)
            }
        }
    }

    /// The authenticator's code for a setup that asked for it. A wrong code leaves `pending` to
    /// try another; any other failure ends it, as [`Setup::enroll`]'s does.
    pub fn second_factor(
        &self,
        pending: &mut Pending,
        code: &Secret,
    ) -> Result<Enrolled, SetupError> {
        let result = self.second_factor_of(pending, code, &mut |_| {});
        if matches!(&result, Err(error) if *error != SetupError::SecondFactorInvalid) {
            self.abandon(pending);
        }
        result
    }

    /// A setup, checked before anything is asked of Oxy or done on the machine: OxyOS's client
    /// configured, the account's id, the password and the PIN usable (a setup without a password,
    /// the QR's, needs a PIN), and its redemption's key made.
    pub fn start(&self, request: Request) -> Result<Attempt, SetupError> {
        if self.platform.is_none() {
            return Err(SetupError::NotConfigured);
        }
        let account = AccountId::parse(&request.account)
            .map_err(|InvalidAccountId| SetupError::InvalidAccount)?;
        match (&request.password, &request.pin) {
            (Some(password), _) if !verifier_password(password) => {
                return Err(SetupError::UnusablePassword)
            }
            (None, None) => return Err(SetupError::PinRequired),
            (password, Some(pin))
                if !usable_pin(pin)
                    || password
                        .as_ref()
                        .is_some_and(|password| password.expose() == pin.expose()) =>
            {
                return Err(SetupError::UnusablePin)
            }
            _ => {}
        }
        let key = RedemptionKey::new().map_err(|error| SetupError::Failed(error.to_string()))?;
        Ok(Attempt {
            account,
            credential: request.credential,
            password: request.password,
            pin_given: request.pin,
            pin: None,
            appearance: request.appearance,
            key,
            profile: None,
            challenge: None,
            held: None,
        })
    }

    /// Sets the account up, reporting each [`Stage`] as it starts, or asks for its
    /// authenticator's code. Again after a failure: with the platform credential the attempt holds,
    /// if it holds one (nothing is asked of Oxy), else from the start.
    pub fn advance(
        &self,
        attempt: &mut Attempt,
        progress: &mut dyn FnMut(Stage),
    ) -> Result<Step, SetupError> {
        let platform = self.platform.clone().ok_or(SetupError::NotConfigured)?;
        if let Some(pin) = attempt.pin_given.take() {
            progress(Stage::Securing);
            attempt.pin = Some(pin_verifier(&pin)?);
        }
        if attempt.held.is_some() {
            return self.take(attempt, None, progress).map(Step::Enrolled);
        }
        attempt.challenge = None;
        // A machine that could not take the account is found out before Oxy is asked.
        {
            let _machine = self.lock();
            self.local(&attempt.account)?;
        }
        // Once: a retry may follow a redemption that reached Oxy (its answer lost), which retired
        // the credential the check mints with. The redemption checks the account again itself.
        let profile = match attempt.profile.clone() {
            Some(profile) => profile,
            None => {
                progress(Stage::Verifying);
                let profile = {
                    let token = self.api.mint_token(&attempt.credential, &attempt.account)?;
                    self.api.profile(&token)?
                };
                if profile.id != attempt.account.as_str() {
                    return Err(SetupError::AccountMismatch);
                }
                attempt.profile = Some(profile.clone());
                profile
            }
        };
        let Some(password) = &attempt.password else {
            // The QR's: the user made before the proof is spent, removed if it is not.
            let prepared = self.prepare(&attempt.account, progress)?;
            progress(Stage::SigningIn);
            let signed_in = match self.redeem(&attempt.credential, &platform, &attempt.key) {
                Ok(signed_in) => signed_in,
                Err(error) => return Err(self.unprepare(&attempt.account, prepared, error.into())),
            };
            return self
                .issued(attempt, Some(prepared), profile, signed_in, progress)
                .map(Step::Enrolled);
        };
        let username = profile.username.clone().ok_or(SetupError::NoUsername)?;
        progress(Stage::SigningIn);
        match self.api.sign_in_password(
            &username,
            password,
            &attempt.credential,
            Some(&platform),
        )? {
            SignInStep::SignedIn(signed_in) => self
                .issued(attempt, None, profile, signed_in, progress)
                .map(Step::Enrolled),
            SignInStep::SecondFactor { challenge } => {
                attempt.challenge = Some(challenge);
                Ok(Step::SecondFactor)
            }
        }
    }

    /// The authenticator's code for a setup that asked for it. A wrong code, or Oxy unreachable,
    /// leaves the challenge to try another (or the same) code.
    pub fn second_factor_of(
        &self,
        attempt: &mut Attempt,
        code: &Secret,
        progress: &mut dyn FnMut(Stage),
    ) -> Result<Enrolled, SetupError> {
        let platform = self.platform.clone().ok_or(SetupError::NotConfigured)?;
        let (Some(challenge), Some(profile)) = (&attempt.challenge, &attempt.profile) else {
            return Err(SetupError::SecondFactorInvalid);
        };
        progress(Stage::SigningIn);
        let signed_in =
            self.api
                .second_factor(challenge, code, &attempt.credential, Some(&platform))?;
        let profile = profile.clone();
        attempt.challenge = None;
        self.issued(attempt, None, profile, signed_in, progress)
    }

    /// Ends what a setup that did not finish still holds: the platform credential it was issued,
    /// ended online (best effort, said if it fails). Its conversation calls it when it ends
    /// without success.
    pub fn abandon(&self, attempt: &mut Attempt) {
        if let Some(credential) = attempt.held.take() {
            self.end_online(&credential, &attempt.account);
        }
    }

    /// The QR's redemption, asked again with the same key while it fails on its way.
    fn redeem(
        &self,
        credential: &DeviceCredential,
        platform: &PlatformClient,
        key: &RedemptionKey,
    ) -> Result<SignedIn, ApiError> {
        let mut delays = self.retry_delays.iter();
        loop {
            match self.api.platform_device_sign_in(credential, platform, key) {
                Err(error) if on_the_way(&error) => match delays.next() {
                    Some(delay) => std::thread::sleep(*delay),
                    None => return Err(error),
                },
                answered => return answered,
            }
        }
    }

    /// A sign-in done: the platform credential it issued kept by `attempt`, then taken by the
    /// machine ([`Setup::take`]).
    fn issued(
        &self,
        attempt: &mut Attempt,
        prepared: Option<Prepared>,
        profile: Profile,
        signed_in: SignedIn,
        progress: &mut dyn FnMut(Stage),
    ) -> Result<Enrolled, SetupError> {
        if signed_in.user_id != attempt.account.as_str()
            || signed_in.device_id != attempt.credential.device_id
        {
            // Not this device's sign-in: whatever it issued is not the machine's to end.
            let error = SetupError::AccountMismatch;
            return Err(match prepared {
                Some(prepared) => self.unprepare(&attempt.account, prepared, error),
                None => error,
            });
        }
        let Some(secret) = signed_in.device_secret else {
            let error = SetupError::Oxy("the sign-in issued no platform credential".into());
            return Err(match prepared {
                Some(prepared) => self.unprepare(&attempt.account, prepared, error),
                None => error,
            });
        };
        attempt.profile = Some(profile);
        attempt.held = Some(DeviceCredential {
            device_id: signed_in.device_id,
            device_secret: secret,
        });
        self.take(attempt, prepared, progress)
    }

    /// The machine takes the account with the platform credential `attempt` holds: its user
    /// (`prepared`, or prepared now), its file, its local sign-in, its administrator, its
    /// verifier. Taken, the credential is the account's file's, and the one an account set up
    /// again had is ended online; not taken, `attempt` keeps it.
    fn take(
        &self,
        attempt: &mut Attempt,
        prepared: Option<Prepared>,
        progress: &mut dyn FnMut(Stage),
    ) -> Result<Enrolled, SetupError> {
        let (Some(credential), Some(profile)) = (&attempt.held, &attempt.profile) else {
            return Err(SetupError::Failed(
                "no platform credential to set up with".into(),
            ));
        };
        let prepared = match prepared {
            Some(prepared) => prepared,
            None => self.prepare(&attempt.account, progress)?,
        };
        progress(Stage::Finishing);
        let (enrolled, replaced) = self.commit(
            &attempt.account,
            prepared,
            credential,
            attempt.password.as_ref(),
            attempt.pin.as_deref(),
            profile.clone(),
            attempt.appearance,
        )?;
        let credential = attempt.held.take().expect("held, as checked");
        if let Some(replaced) = replaced.filter(|old| old.device_id != credential.device_id) {
            self.end_online(&replaced, &attempt.account);
        }
        Ok(enrolled)
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

    /// The user's verifier set to `password`; without one, left as it is (a new user's is locked,
    /// an enrolled account's stays the one it had).
    fn verify(&self, user: &UserName, password: Option<&Secret>) -> Result<(), String> {
        match password {
            Some(password) => self
                .users
                .set_verifier(user, password)
                .map_err(|error| error.to_string()),
            None => Ok(()),
        }
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

    /// The account's user made (`useradd`, its home with it), or the enrolled account's found as
    /// enrolled: the part of the machine's setup that can fail before the credential is taken.
    fn prepare(
        &self,
        account: &AccountId,
        progress: &mut dyn FnMut(Stage),
    ) -> Result<Prepared, SetupError> {
        progress(Stage::Preparing);
        let _machine = self.lock();
        let user = account.user_name();
        match self.local(account)? {
            Local::Enrolled(enrollment) => Ok(Prepared::Enrolled(enrollment)),
            Local::New => match self.users.create(&user) {
                Ok(made) => Ok(Prepared::Made(made)),
                Err(UsersError::Exists) => Err(SetupError::Conflict(format!(
                    "a user {user} exists in another database"
                ))),
                // No user had the name a moment ago, under the lock: whatever useradd left of it
                // is this setup's to remove.
                Err(error) => {
                    let error = error.to_string();
                    Err(match self.users.remove(&user) {
                        Ok(()) | Err(UsersError::Missing) => SetupError::Failed(error),
                        Err(rollback) => SetupError::RollbackFailed {
                            error,
                            rollback: rollback.to_string(),
                        },
                    })
                }
            },
        }
    }

    /// Undoes [`Setup::prepare`] after `error`: a user it made is removed with its home.
    fn unprepare(&self, account: &AccountId, prepared: Prepared, error: SetupError) -> SetupError {
        let Prepared::Made(_) = prepared else {
            return error;
        };
        let _machine = self.lock();
        match self.users.remove(&account.user_name()) {
            Ok(()) | Err(UsersError::Missing) => error,
            Err(rollback) => SetupError::RollbackFailed {
                error: error.to_string(),
                rollback: rollback.to_string(),
            },
        }
    }

    /// Seeds the home of `made`, a user this setup made, with the appearance chosen.
    fn seed(&self, made: &UnixUser, appearance: Option<Appearance>) -> Result<(), String> {
        let Some(appearance) = appearance else {
            return Ok(());
        };
        let mut preferences = oxyos_preferences::Preferences::default();
        preferences
            .set("appearance", appearance.name())
            .map_err(|_| "the appearance is not a preference".to_owned())?;
        let file = oxyos_preferences::path(None, Some(&made.home))
            .and_then(|path| path.strip_prefix(&made.home).ok().map(Path::to_owned))
            .ok_or_else(|| "the preferences are not in the home".to_owned())?;
        self.users
            .seed_home(made, &file, preferences.to_text().as_bytes())
            .map_err(|error| error.to_string())
    }

    /// The machine takes the account (`prepared` for it): its file with `credential`, its local
    /// sign-in (`pin`, a PIN's verifier), a new home's preferences, its administrator, its
    /// verifier. A failure undoes it, the user this setup made removed. The credential an enrolled
    /// account had before, if any, is given back.
    #[allow(clippy::too_many_arguments)]
    fn commit(
        &self,
        account: &AccountId,
        prepared: Prepared,
        credential: &DeviceCredential,
        password: Option<&Secret>,
        pin: Option<&str>,
        profile: Profile,
        appearance: Option<Appearance>,
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
            // A password given signs in; without one, an enrolled account keeps what it had and a
            // new one has the PIN alone.
            password: password.is_some()
                || previous_local.as_ref().is_some_and(|local| local.password)
                    && self.store.get(account).ok().flatten().is_some(),
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
        let uid = match prepared {
            Prepared::Made(made) => {
                // Still the user this setup made, with no account enrolled for it meanwhile.
                let found = self
                    .users
                    .lookup(&user)
                    .map_err(|error| SetupError::Failed(error.to_string()))?;
                let enrolled = self
                    .store
                    .get(account)
                    .map_err(|error| SetupError::Failed(error.to_string()))?;
                if found.map(|found| found.uid) != Some(made.uid) || enrolled.is_some() {
                    return Err(SetupError::Conflict(format!(
                        "{user} changed while it was set up"
                    )));
                }
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
                    .and_then(|()| self.seed(&made, appearance))
                    .and_then(|()| self.administer(&user))
                    .and_then(|administrator| self.verify(&user, password).map(|()| administrator));
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
            Prepared::Enrolled(prepared) => {
                // As it is now: Settings or the platform may have changed it meanwhile.
                let previous = match self.local(account)? {
                    Local::Enrolled(previous) if previous.uid == prepared.uid => previous,
                    _ => {
                        return Err(SetupError::Conflict(format!(
                            "{user} changed while it was set up"
                        )))
                    }
                };
                let uid = previous.uid;
                self.store
                    .put(&enrollment(uid))
                    .map_err(|error| SetupError::Failed(error.to_string()))?;
                let done = self
                    .store
                    .put_local(&user, &local)
                    .map_err(|error| error.to_string())
                    .and_then(|()| self.administer(&user))
                    .and_then(|administrator| self.verify(&user, password).map(|()| administrator));
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

/// Whether a request failed on its way, so that it may or may not have reached Oxy: unreachable,
/// timed out, or a 5xx.
fn on_the_way(error: &ApiError) -> bool {
    matches!(
        error,
        ApiError::Offline(_)
            | ApiError::Refused {
                status: 500..=599,
                ..
            }
    )
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
