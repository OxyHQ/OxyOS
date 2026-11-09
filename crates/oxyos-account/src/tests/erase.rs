//! Erasing the machine (ADR 0054): who may, in what order, failing closed; what Settings is shown
//! first; and `systemctl` itself, a script of the tests' own standing for it. Oxy, the users, the
//! password check and the system's side are fakes; the store is real.

use super::platform::{machine_as, Machine, Service};
use super::*;
use crate::erase::{
    ErasePreview, ErasedAccount, Eraser, Erasing, PasswordCheck, Passwords, Refusal, System,
    Systemctl, ERASE_UNIT,
};
use crate::platform::TokenReason;
use crate::store::Enrollment;
use std::sync::Arc;
use std::time::Duration;

/// Ada's uid in these tests.
const ADA: u32 = 1000;
const GRACE_ACCOUNT: &str = "64f7c2a1b8e9d3f4a1c2b3d4";
const GRACE: u32 = 1001;
const AARON_ACCOUNT: &str = "64f7c2a1b8e9d3f4a1c2b3d5";
const AARON: u32 = 1002;

/// The system's side: whether the erase's unit is installed, whether starting it arms the erase,
/// and what was asked of it, in order. Oxy's calls when the erase was armed are kept, to show
/// nothing was signed out before.
pub(crate) struct FakeSystem {
    pub(crate) installed: bool,
    pub(crate) arming_fails: bool,
    pub(crate) asked: Mutex<Vec<&'static str>>,
    pub(crate) platform: Mutex<Option<Arc<Service>>>,
    pub(crate) oxy_when_armed: Mutex<Option<Vec<Call>>>,
}

impl Default for FakeSystem {
    fn default() -> Self {
        Self {
            installed: true,
            arming_fails: false,
            asked: Mutex::new(Vec::new()),
            platform: Mutex::new(None),
            oxy_when_armed: Mutex::new(None),
        }
    }
}

impl FakeSystem {
    pub(crate) fn asked(&self) -> Vec<&'static str> {
        self.asked.lock().unwrap().clone()
    }
}

impl System for Arc<FakeSystem> {
    fn erase_installed(&self) -> Result<bool, String> {
        self.asked.lock().unwrap().push("installed?");
        Ok(self.installed)
    }

    fn arm_erase(&self) -> Result<(), String> {
        self.asked.lock().unwrap().push("arm");
        if let Some(platform) = &*self.platform.lock().unwrap() {
            *self.oxy_when_armed.lock().unwrap() = Some(platform.setup().api().calls());
        }
        if self.arming_fails {
            return Err("oxyos-erase.service failed".into());
        }
        Ok(())
    }

    fn restart(&self) -> Result<(), String> {
        self.asked.lock().unwrap().push("restart");
        Ok(())
    }
}

/// The account's password as the fake verifier knows it, and what a check answers instead.
pub(crate) struct FakePasswords {
    pub(crate) password: &'static str,
    pub(crate) answer: Option<fn() -> PasswordCheck>,
    pub(crate) checked: Mutex<Vec<String>>,
}

impl Default for FakePasswords {
    fn default() -> Self {
        Self {
            password: PASSWORD,
            answer: None,
            checked: Mutex::new(Vec::new()),
        }
    }
}

impl Passwords for Arc<FakePasswords> {
    fn check(&self, user: &UserName, password: Secret) -> PasswordCheck {
        self.checked.lock().unwrap().push(user.as_str().into());
        if let Some(answer) = self.answer {
            return answer();
        }
        if password.expose() == self.password {
            PasswordCheck::Accepted
        } else {
            PasswordCheck::Rejected
        }
    }
}

pub(crate) fn eraser(system: &Arc<FakeSystem>, passwords: &Arc<FakePasswords>) -> Eraser {
    Eraser::new(Box::new(system.clone()), Box::new(passwords.clone()))
}

struct Erase {
    machine: Machine,
    system: Arc<FakeSystem>,
    passwords: Arc<FakePasswords>,
    eraser: Eraser,
}

impl Erase {
    fn new(api: FakeApi, system: FakeSystem, passwords: FakePasswords) -> Self {
        Self::on(machine_as(api, PASSWORD, Some(ADA)), system, passwords)
    }

    fn on(machine: Machine, system: FakeSystem, passwords: FakePasswords) -> Self {
        let system = Arc::new(system);
        *system.platform.lock().unwrap() = Some(machine.platform.clone());
        let passwords = Arc::new(passwords);
        let eraser = eraser(&system, &passwords);
        Self {
            machine,
            system,
            passwords,
            eraser,
        }
    }

    fn preview(&self, uid: u32) -> ErasePreview {
        self.eraser.preview(&self.machine.platform, uid)
    }

    fn erase(&self, uid: u32, password: &str) -> Result<Erasing, Refusal> {
        self.eraser
            .erase_all(&self.machine.platform, uid, Secret::new(password))
    }

    /// Nothing was armed, nothing signed out, and the platform still mints tokens.
    fn nothing_changed(&self) {
        assert!(!self.system.asked().contains(&"arm"));
        assert!(!self.machine.api().calls().contains(&Call::SignOut));
        assert!(!self.machine.platform.erasing());
        assert!(self
            .machine
            .platform
            .token(ADA, &Attestation::OwnPlatform, TokenReason::Boot)
            .is_some());
    }
}

/// Another account enrolled on the machine (its user made, its file written), signed in on the
/// same device with a holder secret of its own, or signed out (no credential).
fn enroll(
    machine: &Machine,
    account: &str,
    uid: u32,
    username: Option<&str>,
    display_name: Option<&str>,
    signed_in: bool,
) {
    let account = AccountId::parse(account).unwrap();
    let user = account.user_name();
    machine.users().add(user.as_str(), uid);
    machine
        .platform
        .setup()
        .store()
        .put(&Enrollment {
            account: account.clone(),
            user,
            uid,
            profile: Profile {
                id: account.as_str().into(),
                username: username.map(Into::into),
                display_name: display_name.map(Into::into),
                avatar: None,
                languages: None,
            },
            credential: signed_in.then(|| DeviceCredential {
                device_id: DEVICE.into(),
                device_secret: Secret::new(OTHER_DEVICE_SECRET),
            }),
            password_changed: false,
        })
        .unwrap();
}

fn three_accounts() -> FakeApi {
    FakeApi {
        on_device: vec![ACCOUNT, GRACE_ACCOUNT, AARON_ACCOUNT],
        ..FakeApi::default()
    }
}

fn account(name: &str, handle: Option<&str>, signs_out_online: bool) -> ErasedAccount {
    ErasedAccount {
        name: name.into(),
        handle: handle.map(Into::into),
        signs_out_online,
    }
}

#[test]
fn the_first_account_set_up_is_the_administrator() {
    let erase = Erase::new(
        FakeApi::default(),
        FakeSystem::default(),
        FakePasswords::default(),
    );
    assert!(erase
        .machine
        .users()
        .is_admin(super::account().user_name().as_str()));
}

#[test]
fn the_preview_lists_every_account_the_asking_one_first_then_by_name() {
    let erase = Erase::new(
        three_accounts(),
        FakeSystem::default(),
        FakePasswords::default(),
    );
    enroll(
        &erase.machine,
        GRACE_ACCOUNT,
        GRACE,
        Some("grace"),
        Some("Grace Hopper"),
        true,
    );
    // Signed out by Oxy, and with a handle alone.
    enroll(
        &erase.machine,
        AARON_ACCOUNT,
        AARON,
        Some("aaron"),
        None,
        false,
    );
    // Not yet known online: no account is said to be signed out online.
    assert_eq!(
        erase.preview(ADA),
        ErasePreview {
            available: true,
            administrator: true,
            online: false,
            accounts: vec![
                account("Ada Lovelace", Some(USERNAME), false),
                account("aaron", Some("aaron"), false),
                account("Grace Hopper", Some("grace"), false),
            ],
        }
    );
    // Online once Oxy was reached: the accounts with a credential are.
    assert!(erase
        .machine
        .platform
        .token(ADA, &Attestation::OwnPlatform, TokenReason::Boot)
        .is_some());
    let preview = erase.preview(ADA);
    assert!(preview.online);
    assert_eq!(
        preview.accounts,
        [
            account("Ada Lovelace", Some(USERNAME), true),
            account("aaron", Some("aaron"), false),
            account("Grace Hopper", Some("grace"), true),
        ]
    );
    // Grace asking: not an administrator, and only her own account is named to her.
    let preview = erase.preview(GRACE);
    assert!(!preview.administrator);
    // Her own last call, not Ada's, says whether she is online: she made none.
    assert_eq!(
        preview.accounts,
        [account("Grace Hopper", Some("grace"), false)]
    );
    // Asking is no erase.
    assert_eq!(
        erase.system.asked(),
        ["installed?", "installed?", "installed?"]
    );
    assert!(erase.passwords.checked.lock().unwrap().is_empty());
}

#[test]
fn an_account_with_neither_name_nor_handle_is_unnamed_never_named_by_its_user() {
    let erase = Erase::new(
        three_accounts(),
        FakeSystem::default(),
        FakePasswords::default(),
    );
    enroll(&erase.machine, GRACE_ACCOUNT, GRACE, None, Some("  "), true);
    assert_eq!(erase.preview(ADA).accounts[1], account("", None, false));
}

#[test]
fn a_machine_without_its_recovery_is_not_available() {
    let erase = Erase::new(
        FakeApi::default(),
        FakeSystem {
            installed: false,
            ..FakeSystem::default()
        },
        FakePasswords::default(),
    );
    assert!(!erase.preview(ADA).available);
    assert_eq!(erase.erase(ADA, PASSWORD), Err(Refusal::Unavailable));
    erase.nothing_changed();
}

#[test]
fn only_an_administrator_may_erase_and_its_password_is_not_even_checked() {
    let erase = Erase::new(
        three_accounts(),
        FakeSystem::default(),
        FakePasswords::default(),
    );
    enroll(
        &erase.machine,
        GRACE_ACCOUNT,
        GRACE,
        Some("grace"),
        Some("Grace Hopper"),
        true,
    );
    assert_eq!(erase.erase(GRACE, PASSWORD), Err(Refusal::NotAdministrator));
    // A process of no enrolled account's user is no administrator either.
    assert_eq!(erase.erase(4242, PASSWORD), Err(Refusal::NotAdministrator));
    assert!(erase.passwords.checked.lock().unwrap().is_empty());
    assert!(erase.system.asked().is_empty());
    erase.nothing_changed();
}

#[test]
fn a_wrong_password_changes_nothing() {
    let erase = Erase::new(
        FakeApi::default(),
        FakeSystem::default(),
        FakePasswords::default(),
    );
    assert_eq!(erase.erase(ADA, "not it"), Err(Refusal::WrongPassword));
    // Checked as the administrator's own.
    assert_eq!(
        *erase.passwords.checked.lock().unwrap(),
        [super::account().user_name().as_str().to_owned()]
    );
    assert!(erase.system.asked().is_empty());
    erase.nothing_changed();
}

#[test]
fn a_throttled_password_says_when_another_may_be_tried() {
    let erase = Erase::new(
        FakeApi::default(),
        FakeSystem::default(),
        FakePasswords {
            answer: Some(|| PasswordCheck::Throttled(Duration::from_millis(29_200))),
            ..FakePasswords::default()
        },
    );
    assert_eq!(
        erase.erase(ADA, PASSWORD),
        Err(Refusal::Throttled { retry_after: 30 })
    );
    erase.nothing_changed();
}

#[test]
fn a_password_that_cannot_be_checked_changes_nothing() {
    let erase = Erase::new(
        FakeApi::default(),
        FakeSystem::default(),
        FakePasswords {
            answer: Some(|| PasswordCheck::Failed("PAM: broken".into())),
            ..FakePasswords::default()
        },
    );
    assert_eq!(erase.erase(ADA, PASSWORD), Err(Refusal::Failed));
    erase.nothing_changed();
}

#[test]
fn an_erase_its_unit_could_not_arm_signs_nobody_out() {
    let erase = Erase::new(
        FakeApi::default(),
        FakeSystem {
            arming_fails: true,
            ..FakeSystem::default()
        },
        FakePasswords::default(),
    );
    assert_eq!(erase.erase(ADA, PASSWORD), Err(Refusal::Failed));
    assert_eq!(erase.system.asked(), ["installed?", "arm"]);
    assert!(!erase.machine.api().calls().contains(&Call::SignOut));
    assert!(!erase.machine.platform.erasing());
    // Another try may be made.
    assert_eq!(erase.erase(ADA, PASSWORD), Err(Refusal::Failed));
}

#[test]
fn armed_first_then_every_account_signed_out_online() {
    let erase = Erase::new(
        three_accounts(),
        FakeSystem::default(),
        FakePasswords::default(),
    );
    enroll(
        &erase.machine,
        GRACE_ACCOUNT,
        GRACE,
        Some("grace"),
        Some("Grace Hopper"),
        true,
    );
    enroll(
        &erase.machine,
        AARON_ACCOUNT,
        AARON,
        Some("aaron"),
        None,
        false,
    );
    assert_eq!(
        erase.erase(ADA, PASSWORD),
        Ok(Erasing {
            signed_out: vec!["Ada Lovelace".into(), "aaron".into(), "Grace Hopper".into()],
            not_signed_out: vec![],
        })
    );
    // Armed before anything was asked of Oxy; each signed-in account signed out after.
    assert_eq!(erase.system.asked(), ["installed?", "arm"]);
    assert_eq!(*erase.system.oxy_when_armed.lock().unwrap(), Some(vec![]));
    let calls = erase.machine.api().calls();
    assert!(calls.contains(&Call::SignOut));
    assert!(calls
        .iter()
        .all(|call| matches!(call, Call::Mint | Call::SignOut)));
    // Restarting is the caller's, once it answered.
    erase.eraser.restart();
    assert_eq!(erase.system.asked(), ["installed?", "arm", "restart"]);
    // Nothing on the disk was deleted here: the recovery erases it.
    assert_eq!(
        erase
            .machine
            .platform
            .setup()
            .store()
            .users()
            .unwrap()
            .len(),
        3
    );
    assert!(erase
        .machine
        .users()
        .steps()
        .iter()
        .all(|step| *step != UserStep::Remove));
}

#[test]
fn once_armed_nothing_more_is_asked_of_oxy_and_no_second_erase_runs() {
    let erase = Erase::new(
        FakeApi::default(),
        FakeSystem::default(),
        FakePasswords::default(),
    );
    assert!(erase.erase(ADA, PASSWORD).is_ok());
    let calls = erase.machine.api().calls().len();
    assert!(erase.machine.platform.erasing());
    assert!(erase
        .machine
        .platform
        .token(ADA, &Attestation::OwnPlatform, TokenReason::Response401)
        .is_none());
    assert_eq!(erase.machine.api().calls().len(), calls);
    assert_eq!(erase.erase(ADA, PASSWORD), Err(Refusal::Failed));
    assert_eq!(erase.system.asked(), ["installed?", "arm"]);
}

#[test]
fn an_erase_armed_holds_when_the_service_starts_again_before_the_restart() {
    let erase = Erase::new(
        FakeApi::default(),
        FakeSystem::default(),
        FakePasswords::default(),
    );
    assert!(erase.erase(ADA, PASSWORD).is_ok());
    // The service started again over the same accounts (it crashed, say): still armed.
    let again = Erase::on(
        erase.machine.with_api(FakeApi::default()),
        FakeSystem::default(),
        FakePasswords::default(),
    );
    assert!(again.machine.platform.erasing());
    assert!(again
        .machine
        .platform
        .token(ADA, &Attestation::OwnPlatform, TokenReason::Boot)
        .is_none());
    assert!(again.machine.api().calls().is_empty());
    assert_eq!(again.erase(ADA, PASSWORD), Err(Refusal::Failed));
    assert!(again.system.asked().is_empty());
}

#[test]
fn offline_the_erase_goes_on_and_says_who_was_not_signed_out() {
    let online = machine_as(three_accounts(), PASSWORD, Some(ADA));
    let offline = online.with_api(FakeApi {
        offline_from: Some(Call::Mint),
        ..three_accounts()
    });
    let erase = Erase::on(offline, FakeSystem::default(), FakePasswords::default());
    enroll(
        &erase.machine,
        GRACE_ACCOUNT,
        GRACE,
        Some("grace"),
        Some("Grace Hopper"),
        true,
    );
    assert_eq!(
        erase.erase(ADA, PASSWORD),
        Ok(Erasing {
            signed_out: vec![],
            not_signed_out: vec!["Ada Lovelace".into(), "Grace Hopper".into()],
        })
    );
    assert_eq!(erase.system.asked(), ["installed?", "arm"]);
}

#[test]
fn one_account_oxy_refuses_to_sign_out_does_not_stop_the_others() {
    let erase = Erase::new(
        three_accounts(),
        FakeSystem::default(),
        FakePasswords::default(),
    );
    // Grace's credential is one Oxy never issued: its mint is refused as unknown, which is a
    // session already gone; an account whose device Oxy cannot be asked about is not signed out.
    enroll(
        &erase.machine,
        GRACE_ACCOUNT,
        GRACE,
        Some("grace"),
        Some("Grace Hopper"),
        true,
    );
    let mut grace = erase
        .machine
        .platform
        .setup()
        .store()
        .get(&AccountId::parse(GRACE_ACCOUNT).unwrap())
        .unwrap()
        .unwrap();
    grace.credential = Some(DeviceCredential {
        device_id: "another-device".into(),
        device_secret: Secret::new("never-issued"),
    });
    erase.machine.platform.setup().store().put(&grace).unwrap();
    let erasing = erase.erase(ADA, PASSWORD).unwrap();
    assert_eq!(erasing.signed_out, ["Ada Lovelace", "Grace Hopper"]);
    assert!(erasing.not_signed_out.is_empty());
}

#[test]
fn the_preview_and_the_answers_wire_shapes_are_the_documented_ones() {
    let preview = ErasePreview {
        available: true,
        administrator: false,
        online: true,
        accounts: vec![
            account("Ada Lovelace", Some("ada"), true),
            account("oxy-x", None, false),
        ],
    };
    assert_eq!(
        serde_json::to_value(&preview).unwrap(),
        serde_json::json!({
            "available": true,
            "administrator": false,
            "online": true,
            "accounts": [
                {"name": "Ada Lovelace", "handle": "ada", "signsOutOnline": true},
                {"name": "oxy-x", "handle": null, "signsOutOnline": false},
            ],
        })
    );
    assert_eq!(
        serde_json::to_value(Erasing {
            signed_out: vec!["Ada Lovelace".into()],
            not_signed_out: vec!["Grace Hopper".into()],
        })
        .unwrap(),
        serde_json::json!({"signedOut": ["Ada Lovelace"], "notSignedOut": ["Grace Hopper"]})
    );
}

/// A `systemctl` of the test's own: a link to the committed `fake-systemctl`, which writes its
/// arguments to a log beside it, says `load_state` for `show` and exits `start` for `start`.
fn systemctl(dir: &TempDir, load_state: &str, start: i32) -> (Systemctl, PathBuf) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tests/fake-systemctl");
    let link = dir.path().join("systemctl");
    std::os::unix::fs::symlink(fixture, &link).unwrap();
    fs::write(
        dir.path().join("systemctl.load-state"),
        format!("{load_state}\n"),
    )
    .unwrap();
    fs::write(dir.path().join("systemctl.start-status"), start.to_string()).unwrap();
    (Systemctl::at(link), dir.path().join("systemctl.log"))
}

#[test]
fn systemctl_is_asked_for_the_unit_its_start_and_a_restart() {
    let dir = TempDir::new("systemctl");
    let (system, log) = systemctl(&dir, "loaded", 0);
    assert_eq!(system.erase_installed(), Ok(true));
    assert_eq!(system.arm_erase(), Ok(()));
    assert_eq!(system.restart(), Ok(()));
    assert_eq!(
        fs::read_to_string(&log).unwrap(),
        format!(
            "--no-ask-password show --property=LoadState --value {ERASE_UNIT}\n\
             --no-ask-password start {ERASE_UNIT}\n\
             --no-ask-password reboot\n"
        )
    );
}

#[test]
fn a_unit_not_installed_or_failing_is_said() {
    let dir = TempDir::new("systemctl-absent");
    let (system, _) = systemctl(&dir, "not-found", 1);
    assert_eq!(system.erase_installed(), Ok(false));
    assert!(system.arm_erase().is_err());
    let missing = Systemctl::at(dir.path().join("no-systemctl"));
    assert!(missing.erase_installed().is_err());
}
