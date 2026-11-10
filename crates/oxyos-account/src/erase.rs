//! Erasing the machine: Settings' *Erase All Content and Settings…*
//! ([ADR 0054](../../docs/adr/0054-recovery-and-erasing.mdx)), what `oxyos-accountd` does of it.
//! The recovery erases the disk at the next boot; this service only authorizes the erase, arms it,
//! signs the accounts out of this device online and restarts the machine.
//!
//! - **Who may.** Settings registered as a system application ([`session`](crate::session) checks
//!   that), for an account that is one of the machine's administrators
//!   ([`ADMINISTRATORS`](crate::users::ADMINISTRATORS)), with that account's own password, checked
//!   on the machine against its verifier, as the lock checks one, and throttled as the lock is
//!   ([`Passwords`], the service's PAM). A password only Oxy would accept online is no
//!   authorization here: erasing is a local administrator's decision.
//! - **Arming** is `oxyos-erase.service`'s ([`ERASE_UNIT`], installed with the recovery): it runs
//!   with its own privileges, writes the erase request the recovery carries out at the next boot
//!   and makes the next boot start the recovery. This service never writes to the EFI system
//!   partition or the firmware's variables. A machine without the unit cannot be erased from
//!   Settings.
//! - **In order, failing closed:** the administrator, its password, the unit armed. Anything that
//!   fails before the unit is armed changes nothing; once it is armed, the machine is erased at its
//!   next boot whatever follows. Then every enrolled account is signed out of this device online
//!   where it can be, at once and each within the HTTP client's own bounds, and one that cannot be
//!   (offline, refused) does not stop the erase: its device session ends with the erased
//!   credential and expires at Oxy. Last the answer, and the machine restarts
//!   ([`Eraser::restart`]). Nothing on the disk is deleted here.
//!
//! Each of the system's side ([`System`], `systemctl`) and the password check ([`Passwords`]) is a
//! seam: the tests run every rule without root.

use crate::api::OxyApi;
use crate::platform::Platform;
use crate::secret::Secret;
use crate::store::Enrollment;
use crate::users::Users;
use crate::UserName;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;
// The messages, the protocol's own crate's (`oxyos-account-proto`): clients link them alone.
pub use oxyos_account_proto::erase::{ErasePreview, ErasedAccount, Erasing};

/// The unit that arms the erase, installed with the recovery.
pub const ERASE_UNIT: &str = "oxyos-erase.service";

/// Why an erase was refused. Nothing was changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The asking account is not one of the machine's administrators.
    NotAdministrator,
    /// The password is not the administrator's.
    WrongPassword,
    /// Too many wrong passwords: another may be tried in this many seconds.
    Throttled { retry_after: u64 },
    /// The machine has no recovery to erase it ([`ERASE_UNIT`] is not installed).
    Unavailable,
    /// Something failed before the erase was armed.
    Failed,
}

/// What a password check came to.
#[derive(Debug, PartialEq, Eq)]
pub enum PasswordCheck {
    Accepted,
    Rejected,
    /// Not checked: too many rejections, another may be tried after this long.
    Throttled(Duration),
    /// The check could not be made (PAM broken): no judgement on the password, and not counted.
    Failed(String),
}

/// The account's own password, checked on the machine against its verifier and throttled.
pub trait Passwords {
    fn check(&self, user: &UserName, password: Secret) -> PasswordCheck;
}

/// The system's side of erasing: the unit and restarting.
pub trait System {
    /// Whether [`ERASE_UNIT`] is installed (loaded).
    fn erase_installed(&self) -> Result<bool, String>;
    /// Starts [`ERASE_UNIT`] and waits for it: Ok once it ended well, the erase armed.
    fn arm_erase(&self) -> Result<(), String>;
    /// Restarts the machine.
    fn restart(&self) -> Result<(), String>;
}

/// The system's own: systemd, through `systemctl`, run from an argument vector with a clean
/// environment, as the users' tools are.
pub struct Systemctl {
    pub(crate) systemctl: PathBuf,
}

impl Default for Systemctl {
    fn default() -> Self {
        Self::new()
    }
}

impl Systemctl {
    pub fn new() -> Self {
        Self::at("/usr/bin/systemctl".into())
    }

    /// Another `systemctl` (the tests' own).
    pub fn at(systemctl: PathBuf) -> Self {
        Self { systemctl }
    }

    fn run(&self, arguments: &[&str]) -> Result<std::process::Output, String> {
        Command::new(&self.systemctl)
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .arg("--no-ask-password")
            .args(arguments)
            .output()
            .map_err(|error| format!("systemctl cannot run: {error}"))
    }
}

fn failed(what: &str, output: &std::process::Output) -> String {
    format!(
        "systemctl {what} failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

impl System for Systemctl {
    fn erase_installed(&self) -> Result<bool, String> {
        let output = self.run(&["show", "--property=LoadState", "--value", ERASE_UNIT])?;
        if !output.status.success() {
            return Err(failed("show", &output));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim() == "loaded")
    }

    fn arm_erase(&self) -> Result<(), String> {
        let output = self.run(&["start", ERASE_UNIT])?;
        if output.status.success() {
            Ok(())
        } else {
            Err(failed("start", &output))
        }
    }

    fn restart(&self) -> Result<(), String> {
        let output = self.run(&["reboot"])?;
        if output.status.success() {
            Ok(())
        } else {
            Err(failed("reboot", &output))
        }
    }
}

/// The accounts' names and the asking one first, then by name: the preview's order.
fn ordered(mut enrolled: Vec<Enrollment>, uid: u32) -> Vec<Enrollment> {
    enrolled.sort_by(|a, b| {
        (a.uid != uid)
            .cmp(&(b.uid != uid))
            .then_with(|| name_of(a).to_lowercase().cmp(&name_of(b).to_lowercase()))
            .then_with(|| a.user.as_str().cmp(b.user.as_str()))
    });
    enrolled
}

/// What the person is called ([`crate::api::Profile::name`]); empty when the account has no name,
/// never its user's.
fn name_of(enrollment: &Enrollment) -> String {
    enrollment.profile.name().unwrap_or_default().to_owned()
}

/// Erasing the machine, Settings' requests on the session socket.
pub struct Eraser {
    system: Box<dyn System + Send + Sync>,
    passwords: Box<dyn Passwords + Send + Sync>,
    /// One erase at a time.
    under_way: Mutex<()>,
}

impl Eraser {
    pub fn new(
        system: Box<dyn System + Send + Sync>,
        passwords: Box<dyn Passwords + Send + Sync>,
    ) -> Self {
        Self {
            system,
            passwords,
            under_way: Mutex::new(()),
        }
    }

    /// The account's own password checked on the machine (Settings' other changes that need it:
    /// the local sign-in, `crate::pin`).
    pub fn passwords(&self) -> &(dyn Passwords + Send + Sync) {
        &*self.passwords
    }

    /// What erasing would do, for `uid`'s account. A machine whose unit cannot be asked about is
    /// not available.
    pub fn preview<A, U>(&self, platform: &Platform<A, U>, uid: u32) -> ErasePreview
    where
        A: OxyApi + Send + Sync + 'static,
        U: Users + Send + Sync + 'static,
    {
        let available = match self.system.erase_installed() {
            Ok(installed) => installed,
            Err(error) => {
                eprintln!("oxyos-accountd: whether this machine can be erased: {error}");
                false
            }
        };
        let asking = platform.enrollment_of(uid).ok().flatten();
        let administrator = asking
            .as_ref()
            .is_some_and(|enrollment| is_administrator(platform, &enrollment.user));
        let online = asking
            .as_ref()
            .is_some_and(|enrollment| platform.online(&enrollment.user));
        let enrolled = match (administrator, platform.setup().store().enrollments()) {
            (true, Ok(enrolled)) => enrolled,
            (false, _) => asking.into_iter().collect(),
            (true, Err(error)) => {
                eprintln!("oxyos-accountd: the enrolled accounts cannot be read: {error}");
                Vec::new()
            }
        };
        ErasePreview {
            available,
            administrator,
            online,
            accounts: ordered(enrolled, uid)
                .iter()
                .map(|enrollment| ErasedAccount {
                    name: name_of(enrollment),
                    handle: enrollment.profile.username.clone(),
                    signs_out_online: enrollment.credential.is_some() && online,
                })
                .collect(),
        }
    }

    /// Erases the machine for `uid`'s account, with its password: the account an administrator,
    /// the password its own, the unit armed; then every account signed out of this device online
    /// where it can be. Refused, nothing was changed. The caller answers, then calls
    /// [`Eraser::restart`].
    pub fn erase_all<A, U>(
        &self,
        platform: &Platform<A, U>,
        uid: u32,
        password: Secret,
    ) -> Result<Erasing, Refusal>
    where
        A: OxyApi + Send + Sync + 'static,
        U: Users + Send + Sync + 'static,
    {
        let _one = self.under_way.lock().unwrap_or_else(|p| p.into_inner());
        if platform.erasing() {
            eprintln!("oxyos-accountd: uid {uid}: the erase is armed already");
            return Err(Refusal::Failed);
        }
        // 1. An administrator's own account.
        let enrollment = match platform.enrollment_of(uid) {
            Ok(Some(enrollment)) => enrollment,
            Ok(None) => return Err(Refusal::NotAdministrator),
            Err(error) => {
                eprintln!("oxyos-accountd: uid {uid}'s account cannot be read: {error}");
                return Err(Refusal::Failed);
            }
        };
        match platform.setup().users().is_administrator(&enrollment.user) {
            Ok(true) => {}
            Ok(false) => return Err(Refusal::NotAdministrator),
            Err(error) => {
                eprintln!(
                    "oxyos-accountd: whether {} is an administrator: {error}",
                    enrollment.user
                );
                return Err(Refusal::Failed);
            }
        }
        // 2. Its own password, on the machine.
        match self.passwords.check(&enrollment.user, password) {
            PasswordCheck::Accepted => {}
            PasswordCheck::Rejected => return Err(Refusal::WrongPassword),
            PasswordCheck::Throttled(wait) => {
                return Err(Refusal::Throttled {
                    retry_after: wait.as_secs() + u64::from(wait.subsec_nanos() > 0),
                })
            }
            PasswordCheck::Failed(error) => {
                eprintln!(
                    "oxyos-accountd: {}'s password cannot be checked: {error}",
                    enrollment.user
                );
                return Err(Refusal::Failed);
            }
        }
        // 3. Armed, by the unit.
        match self.system.erase_installed() {
            Ok(true) => {}
            Ok(false) => return Err(Refusal::Unavailable),
            Err(error) => {
                eprintln!("oxyos-accountd: whether this machine can be erased: {error}");
                return Err(Refusal::Failed);
            }
        }
        if let Err(error) = self.system.arm_erase() {
            eprintln!("oxyos-accountd: the erase could not be armed: {error}; nothing changed");
            return Err(Refusal::Failed);
        }
        platform.begin_erasing();
        eprintln!(
            "oxyos-accountd: {} erased this machine: the recovery erases it at the next boot",
            enrollment.user
        );
        // 4. Every account signed out of this device online, at once; none stops the erase.
        let enrolled = ordered(
            platform.setup().store().enrollments().unwrap_or_default(),
            uid,
        );
        let results: Vec<(String, Result<(), String>)> = std::thread::scope(|scope| {
            let signing_out: Vec<_> = enrolled
                .iter()
                .map(|enrollment| {
                    (
                        name_of(enrollment),
                        scope.spawn(|| platform.sign_out_for_erase(&enrollment.user)),
                    )
                })
                .collect();
            signing_out
                .into_iter()
                .map(|(name, signing_out)| {
                    let result = signing_out
                        .join()
                        .unwrap_or_else(|_| Err("its sign-out failed".into()));
                    (name, result)
                })
                .collect()
        });
        let mut erasing = Erasing {
            signed_out: Vec::new(),
            not_signed_out: Vec::new(),
        };
        for ((name, result), enrollment) in results.into_iter().zip(&enrolled) {
            match result {
                Ok(()) => erasing.signed_out.push(name),
                Err(error) => {
                    eprintln!(
                        "oxyos-accountd: {} was not signed out at Oxy: {error}; its device session \
                         ends with the erased credential",
                        enrollment.user
                    );
                    erasing.not_signed_out.push(name);
                }
            }
        }
        Ok(erasing)
    }

    /// Restarts the machine into the recovery, once the erase was answered.
    pub fn restart(&self) {
        if let Err(error) = self.system.restart() {
            eprintln!(
                "oxyos-accountd: the machine could not be restarted: {error}; the erase stays \
                 armed for its next boot"
            );
        }
    }
}

fn is_administrator<A, U>(platform: &Platform<A, U>, user: &UserName) -> bool
where
    A: OxyApi + Send + Sync + 'static,
    U: Users + Send + Sync + 'static,
{
    match platform.setup().users().is_administrator(user) {
        Ok(administrator) => administrator,
        Err(error) => {
            eprintln!("oxyos-accountd: whether {user} is an administrator: {error}");
            false
        }
    }
}
