//! The local sign-in (`crate::pin`): a PIN set up with the account, checked offline with its own
//! throttle and lockout, never sent to Oxy; the password that clears a lockout; Settings' changes,
//! each proved and refused whole.

use super::platform::{machine, Machine};
use super::*;
use crate::erase::{PasswordCheck, Passwords};
use crate::pin::{LocalSignIn, PinCheck, PinChecks, PinRecord, PinState};
use crate::platform::{Checked, Local, LocalChange, LocalRefusal, LocalView, Proof};
use crate::setup::{Progress, Request, Setup, SetupError};
use crate::store::Store;
use std::time::{Duration, Instant};

const PIN: &str = "482916";

fn request(password: Option<&str>, pin: Option<&str>) -> Request {
    Request {
        credential: credential(),
        account: ACCOUNT.into(),
        password: password.map(Secret::new),
        pin: pin.map(Secret::new),
    }
}

fn setup() -> (Setup<FakeApi, FakeUsers>, TempDir) {
    let dir = TempDir::new("pin");
    let store = Store::open(&dir.path().join("accounts")).unwrap();
    let setup = Setup::new(FakeApi::default(), FakeUsers::default(), store)
        .with_platform(platform_client());
    (setup, dir)
}

fn name() -> UserName {
    account().user_name()
}

/// The machine's own password check, as PAM would make it against the fake verifier.
struct VerifierPasswords<'a>(&'a FakeUsers);

impl Passwords for VerifierPasswords<'_> {
    fn check(&self, user: &UserName, password: Secret) -> PasswordCheck {
        let locked = self.0.locked.lock().unwrap().contains(user.as_str());
        match self.0.verifier(user) {
            Some(verifier) if !locked && verifier == password.expose() => PasswordCheck::Accepted,
            _ => PasswordCheck::Rejected,
        }
    }
}

/// Ada, set up with her password and `PIN`.
fn with_pin() -> Machine {
    let machine = machine(FakeApi::default());
    let store = machine.platform.setup().store();
    store
        .put_local(
            &name(),
            &LocalSignIn {
                pin: Some(PinRecord {
                    verifier: crate::pin::hash_pin(&Secret::new(PIN)).unwrap(),
                    failures: 0,
                }),
                password: true,
            },
        )
        .unwrap();
    machine
}

#[test]
fn a_pin_given_at_setup_becomes_its_verifier_and_is_listed() {
    let (setup, _dir) = setup();
    match setup.enroll(request(Some(PASSWORD), Some(PIN))).unwrap() {
        Progress::Enrolled(_) => {}
        other => panic!("{other:?}"),
    }
    // Oxy saw the password, as before, and never the PIN.
    assert_eq!(
        setup.api().calls(),
        vec![Call::Mint, Call::Profile, Call::Password]
    );
    let local = setup.store().local(&name()).unwrap();
    assert!(local.password);
    let record = local.pin.unwrap();
    assert_eq!(record.failures, 0);
    assert!(record.verifier.starts_with("$argon2id$"));
    assert!(crate::pin::verify_pin(&record.verifier, &Secret::new(PIN)).unwrap());
    // Its file is root's alone, as the account's.
    let path = setup.store().directory().join(format!("{}.local", name()));
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains(PIN));
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let (listed, _) = setup.store().list().unwrap();
    assert_eq!(listed[0].pin, PinState::Set);
    assert!(listed[0].password);
}

#[test]
fn an_account_without_a_password_is_refused_before_oxy_is_asked() {
    let (setup, _dir) = setup();
    assert_eq!(
        setup.enroll(request(None, Some(PIN))).unwrap_err(),
        SetupError::PasswordRequired
    );
    assert!(setup.api().calls().is_empty());
    assert!(setup.users().names().is_empty());
    assert_eq!(
        std::fs::read_dir(setup.store().directory())
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn an_unusable_pin_is_refused_before_oxy_is_asked() {
    for pin in ["123", "12a456", "12345678901234567", PASSWORD] {
        let (setup, _dir) = setup();
        let password = if pin == PASSWORD { "4829160" } else { PASSWORD };
        let pin = if pin == PASSWORD { "4829160" } else { pin };
        assert_eq!(
            setup
                .enroll(request(Some(password), Some(pin)))
                .unwrap_err(),
            SetupError::UnusablePin,
            "{pin}"
        );
        assert!(setup.api().calls().is_empty());
        assert!(setup.users().names().is_empty());
    }
}

#[test]
fn a_failed_setup_leaves_no_pin_behind() {
    let (setup, _dir) = setup();
    setup
        .users()
        .failing
        .lock()
        .unwrap()
        .insert(UserStep::SetVerifier);
    assert!(setup.enroll(request(Some(PASSWORD), Some(PIN))).is_err());
    assert_eq!(
        std::fs::read_dir(setup.store().directory())
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn set_up_again_without_a_pin_keeps_it_and_forgets_its_wrong_attempts() {
    let (setup, _dir) = setup();
    setup.enroll(request(Some(PASSWORD), Some(PIN))).unwrap();
    let mut local = setup.store().local(&name()).unwrap();
    local.pin.as_mut().unwrap().failures = 10;
    setup.store().put_local(&name(), &local).unwrap();
    let mut again = request(Some(PASSWORD), None);
    again.credential.device_secret = Secret::new(OTHER_DEVICE_SECRET);
    setup.enroll(again).unwrap();
    let local = setup.store().local(&name()).unwrap();
    let record = local.pin.unwrap();
    assert_eq!(record.failures, 0);
    assert!(crate::pin::verify_pin(&record.verifier, &Secret::new(PIN)).unwrap());
}

#[test]
fn five_wrong_pins_are_free_then_each_waits_and_the_tenth_locks_it_out() {
    let machine = with_pin();
    let store = machine.platform.setup().store();
    let checks = PinChecks::new();
    let start = Instant::now();
    let wrong = Secret::new("000000");
    for n in 1..=4 {
        assert_eq!(
            checks.check(store, &name(), &wrong, start).unwrap(),
            PinCheck::Rejected {
                wait: Duration::ZERO,
                locked: false
            },
            "{n}"
        );
    }
    assert_eq!(
        checks.check(store, &name(), &wrong, start).unwrap(),
        PinCheck::Rejected {
            wait: Duration::from_secs(30),
            locked: false
        }
    );
    // Not even the right one is checked while it waits.
    assert_eq!(
        checks
            .check(
                store,
                &name(),
                &Secret::new(PIN),
                start + Duration::from_secs(29)
            )
            .unwrap(),
        PinCheck::Throttled(Duration::from_secs(1))
    );
    let mut now = start + Duration::from_secs(30);
    for n in 6..=9 {
        let PinCheck::Rejected {
            wait,
            locked: false,
        } = checks.check(store, &name(), &wrong, now).unwrap()
        else {
            panic!("{n}")
        };
        now += wait;
    }
    assert_eq!(
        checks.check(store, &name(), &wrong, now).unwrap(),
        PinCheck::Rejected {
            wait: Duration::ZERO,
            locked: true
        }
    );
    // Locked out: nothing is checked, and the count survives the service (a new PinChecks).
    let later = now + Duration::from_secs(24 * 3600);
    assert_eq!(
        PinChecks::new()
            .check(store, &name(), &Secret::new(PIN), later)
            .unwrap(),
        PinCheck::Locked
    );
    assert_eq!(store.list().unwrap().0[0].pin, PinState::Locked);
}

#[test]
fn a_service_started_again_waits_from_its_start() {
    let machine = with_pin();
    let store = machine.platform.setup().store();
    let mut local = store.local(&name()).unwrap();
    local.pin.as_mut().unwrap().failures = 5;
    store.put_local(&name(), &local).unwrap();
    let start = Instant::now();
    let checks = PinChecks::new();
    assert_eq!(
        checks
            .check(store, &name(), &Secret::new(PIN), start)
            .unwrap(),
        PinCheck::Throttled(Duration::from_secs(30))
    );
    assert_eq!(
        checks
            .check(
                store,
                &name(),
                &Secret::new(PIN),
                start + Duration::from_secs(30)
            )
            .unwrap(),
        PinCheck::Accepted
    );
    assert_eq!(store.local(&name()).unwrap().pin.unwrap().failures, 0);
}

#[test]
fn what_is_no_pin_is_no_guess_and_an_account_without_one_checks_nothing() {
    let machine = with_pin();
    let store = machine.platform.setup().store();
    let checks = PinChecks::new();
    for _ in 0..20 {
        assert!(matches!(
            checks
                .check(store, &name(), &Secret::new(PASSWORD), Instant::now())
                .unwrap(),
            PinCheck::Rejected { locked: false, .. }
        ));
    }
    assert_eq!(store.local(&name()).unwrap().pin.unwrap().failures, 0);
    let without = machine_without_pin();
    assert_eq!(
        without
            .platform
            .check_pin(&name().to_string(), &Secret::new(PIN)),
        PinCheck::NoPin
    );
    assert_eq!(
        with_pin()
            .platform
            .check_pin("oxy-nobody", &Secret::new(PIN)),
        PinCheck::NoPin
    );
}

fn machine_without_pin() -> Machine {
    machine(FakeApi::default())
}

#[test]
fn a_pin_is_never_sent_to_oxy() {
    let machine = with_pin();
    machine.api().calls.lock().unwrap().clear();
    assert_eq!(
        machine
            .platform
            .check_pin(&name().to_string(), &Secret::new(PIN)),
        PinCheck::Accepted
    );
    assert_eq!(
        machine
            .platform
            .check_pin(&name().to_string(), &Secret::new("111111")),
        PinCheck::Rejected {
            wait: Duration::ZERO,
            locked: false
        }
    );
    assert!(machine.api().calls().is_empty());
}

#[test]
fn the_password_signing_in_unlocks_a_locked_out_pin() {
    let machine = with_pin();
    let store = machine.platform.setup().store();
    let mut local = store.local(&name()).unwrap();
    local.pin.as_mut().unwrap().failures = 10;
    store.put_local(&name(), &local).unwrap();
    assert_eq!(
        machine
            .platform
            .check(&name().to_string(), Secret::new(PASSWORD), Local::Accepted),
        Checked::ACCEPTED
    );
    machine.platform.settle();
    assert_eq!(store.local(&name()).unwrap().pin_state(), PinState::Set);
    // A wrong password unlocks nothing.
    local.pin.as_mut().unwrap().failures = 10;
    store.put_local(&name(), &local).unwrap();
    let rejected =
        machine
            .platform
            .check(&name().to_string(), Secret::new("wrong"), Local::Rejected);
    assert_eq!(rejected.result, crate::platform::Verdict::Rejected);
    assert_eq!(store.local(&name()).unwrap().pin_state(), PinState::Locked);
}

fn change(machine: &Machine, proof: Proof, change: LocalChange) -> Result<LocalView, LocalRefusal> {
    let passwords = VerifierPasswords(machine.users());
    machine
        .platform
        .change_local(machine.uid(), proof, change, &passwords)
}

#[test]
fn settings_sets_changes_and_removes_the_pin_proved_by_the_password_or_the_pin() {
    let machine = machine_without_pin();
    let password = || Proof::Password(Secret::new(PASSWORD));
    // A wrong password changes nothing.
    assert_eq!(
        change(
            &machine,
            Proof::Password(Secret::new("wrong")),
            LocalChange::SetPin(Secret::new(PIN))
        ),
        Err(LocalRefusal::WrongPassword)
    );
    assert_eq!(
        change(&machine, password(), LocalChange::SetPin(Secret::new("12"))),
        Err(LocalRefusal::UnusablePin)
    );
    assert_eq!(
        machine.platform.local_sign_in(machine.uid()).unwrap(),
        Some(LocalView {
            pin: PinState::None,
            password: true
        })
    );
    assert_eq!(
        change(&machine, password(), LocalChange::SetPin(Secret::new(PIN))),
        Ok(LocalView {
            pin: PinState::Set,
            password: true
        })
    );
    // Changed with the PIN itself.
    assert_eq!(
        change(
            &machine,
            Proof::Pin(Secret::new("999999")),
            LocalChange::SetPin(Secret::new("1357"))
        ),
        Err(LocalRefusal::WrongPin)
    );
    assert_eq!(
        change(
            &machine,
            Proof::Pin(Secret::new(PIN)),
            LocalChange::SetPin(Secret::new("1357"))
        ),
        Ok(LocalView {
            pin: PinState::Set,
            password: true
        })
    );
    assert_eq!(
        machine
            .platform
            .check_pin(&name().to_string(), &Secret::new("1357")),
        PinCheck::Accepted
    );
    // Removed, with the password alone.
    assert_eq!(
        change(&machine, password(), LocalChange::RemovePin),
        Ok(LocalView {
            pin: PinState::None,
            password: true
        })
    );
    // Nothing was asked of Oxy.
    assert!(machine
        .api()
        .calls()
        .iter()
        .all(|call| *call != Call::Password));
}

#[test]
fn the_password_turned_off_leaves_the_pin_alone_and_on_again_asks_oxy() {
    let machine = with_pin();
    // Not without a PIN, and the PIN cannot go while the password is off.
    let without = machine_without_pin();
    assert_eq!(
        change(
            &without,
            Proof::Password(Secret::new(PASSWORD)),
            LocalChange::PasswordSignIn(false)
        ),
        Err(LocalRefusal::NoPin)
    );
    assert!(!without
        .users()
        .locked
        .lock()
        .unwrap()
        .contains(name().as_str()));
    assert_eq!(
        change(
            &machine,
            Proof::Pin(Secret::new(PIN)),
            LocalChange::PasswordSignIn(false)
        ),
        Ok(LocalView {
            pin: PinState::Set,
            password: false
        })
    );
    assert!(machine
        .users()
        .locked
        .lock()
        .unwrap()
        .contains(name().as_str()));
    assert_eq!(
        change(
            &machine,
            Proof::Pin(Secret::new(PIN)),
            LocalChange::RemovePin
        ),
        Err(LocalRefusal::PasswordSignInOff)
    );
    // The password signs nothing in now, even one Oxy would accept: Oxy is not asked.
    machine.api().calls.lock().unwrap().clear();
    assert_eq!(
        machine
            .platform
            .check(&name().to_string(), Secret::new(PASSWORD), Local::Rejected),
        Checked::REJECTED
    );
    assert!(machine.api().calls().is_empty());
    assert!(machine
        .users()
        .locked
        .lock()
        .unwrap()
        .contains(name().as_str()));
    // On again: the password itself, checked with Oxy, becomes the verifier.
    assert_eq!(
        change(
            &machine,
            Proof::Pin(Secret::new(PIN)),
            LocalChange::PasswordSignIn(true)
        ),
        Err(LocalRefusal::WrongPassword)
    );
    assert_eq!(
        change(
            &machine,
            Proof::Password(Secret::new("wrong")),
            LocalChange::PasswordSignIn(true)
        ),
        Err(LocalRefusal::WrongPassword)
    );
    assert_eq!(
        change(
            &machine,
            Proof::Password(Secret::new(PASSWORD)),
            LocalChange::PasswordSignIn(true)
        ),
        Ok(LocalView {
            pin: PinState::Set,
            password: true
        })
    );
    assert!(!machine
        .users()
        .locked
        .lock()
        .unwrap()
        .contains(name().as_str()));
    assert_eq!(machine.users().verifier(&name()).as_deref(), Some(PASSWORD));
}

#[test]
fn a_locked_out_pin_proves_nothing_and_the_password_unlocks_it() {
    let machine = with_pin();
    let store = machine.platform.setup().store();
    let mut local = store.local(&name()).unwrap();
    local.pin.as_mut().unwrap().failures = 10;
    store.put_local(&name(), &local).unwrap();
    assert_eq!(
        change(
            &machine,
            Proof::Pin(Secret::new(PIN)),
            LocalChange::SetPin(Secret::new("1357"))
        ),
        Err(LocalRefusal::PinLocked)
    );
    assert_eq!(
        change(
            &machine,
            Proof::Password(Secret::new(PASSWORD)),
            LocalChange::SetPin(Secret::new("1357"))
        ),
        Ok(LocalView {
            pin: PinState::Set,
            password: true
        })
    );
}
