use super::*;
use crate::setup::{Enrolled, Progress, Request, Setup, SetupError};
use crate::store::Store;
use crate::users::SystemUsers;

struct Machine<U: Users> {
    setup: Setup<FakeApi, U>,
    _dir: TempDir,
}

fn machine(api: FakeApi) -> Machine<FakeUsers> {
    machine_with(api, FakeUsers::default())
}

fn machine_with<U: Users>(api: FakeApi, users: U) -> Machine<U> {
    let dir = TempDir::new("setup");
    let store = Store::open(&dir.path().join("accounts")).unwrap();
    Machine {
        setup: Setup::new(api, users, store).with_platform(platform_client()),
        _dir: dir,
    }
}

impl<U: Users> Machine<U> {
    fn api(&self) -> &FakeApi {
        self.setup.api()
    }

    fn enrolled_accounts(&self) -> Vec<String> {
        self.setup.store().users().unwrap()
    }

    fn files(&self) -> usize {
        std::fs::read_dir(self.setup.store().directory())
            .unwrap()
            .count()
    }
}

fn request(password: &str) -> Request {
    Request {
        credential: credential(),
        account: ACCOUNT.into(),
        password: Some(Secret::new(password)),
        pin: None,
    }
}

fn enrolled(progress: Result<Progress, SetupError>) -> Enrolled {
    match progress {
        Ok(Progress::Enrolled(enrolled)) => enrolled,
        other => panic!("not enrolled: {other:?}"),
    }
}

/// Nothing of the account on the machine: no user, no file, not even a half-written one.
fn nothing_left(machine: &Machine<FakeUsers>) {
    assert!(machine.setup.users().names().is_empty(), "a user was left");
    assert_eq!(machine.files(), 0, "a file was left");
}

#[test]
fn an_account_is_set_up_with_its_user_verifier_and_credential() {
    let machine = machine(FakeApi::default());
    let enrolled = enrolled(machine.setup.enroll(request(PASSWORD)));
    let name = account().user_name();
    assert_eq!(enrolled.account, account());
    assert_eq!(enrolled.user, name);
    assert_eq!(enrolled.profile.username.as_deref(), Some(USERNAME));
    assert_eq!(
        enrolled.profile.display_name.as_deref(),
        Some("Ada Lovelace")
    );
    assert_eq!(
        machine.api().calls(),
        vec![Call::Mint, Call::Profile, Call::Password]
    );
    assert_eq!(
        machine.setup.users().verifier(&name).as_deref(),
        Some(PASSWORD)
    );
    // The credential kept is the platform credential the sign-in issued, which only the service
    // saw; the one handed over is retired.
    let stored = machine.setup.store().get(&account()).unwrap().unwrap();
    assert_eq!(
        stored.credential,
        Some(DeviceCredential {
            device_id: DEVICE.into(),
            device_secret: Secret::new(platform_secret(1)),
        })
    );
    assert!(!stored.password_changed);
    assert!(!machine
        .api()
        .secrets
        .lock()
        .unwrap()
        .contains(&DEVICE_SECRET.to_string()));
    assert_eq!(
        *machine.api().platforms.lock().unwrap(),
        [Some(platform_client())]
    );
    assert_eq!(stored.uid, enrolled.uid);
    // The verifier is set last.
    assert_eq!(
        machine.setup.users().steps().last(),
        Some(&UserStep::SetVerifier)
    );
}

#[test]
fn offline_setup_is_refused_and_leaves_nothing() {
    for from in [Call::Mint, Call::Profile, Call::Password] {
        let machine = machine(FakeApi {
            offline_from: Some(from),
            ..FakeApi::default()
        });
        assert_eq!(
            machine.setup.enroll(request(PASSWORD)).unwrap_err(),
            SetupError::Offline,
            "offline from {from:?}"
        );
        nothing_left(&machine);
        assert!(!machine
            .setup
            .users()
            .steps()
            .iter()
            .any(|step| *step != UserStep::Lookup));
    }
}

#[test]
fn offline_at_the_second_factor_leaves_nothing() {
    let machine = machine(FakeApi {
        authenticator: true,
        offline_from: Some(Call::SecondFactor),
        ..FakeApi::default()
    });
    let Ok(Progress::SecondFactor(pending)) = machine.setup.enroll(request(PASSWORD)) else {
        panic!("a second factor is asked for");
    };
    assert_eq!(
        machine
            .setup
            .second_factor(&pending, &Secret::new(CODE))
            .unwrap_err(),
        SetupError::Offline
    );
    nothing_left(&machine);
}

#[test]
fn a_wrong_password_is_refused_and_leaves_nothing() {
    let machine = machine(FakeApi::default());
    assert_eq!(
        machine
            .setup
            .enroll(request("correct horse battery"))
            .unwrap_err(),
        SetupError::WrongPassword
    );
    nothing_left(&machine);
}

#[test]
fn oxys_lockout_is_passed_on_with_its_wait() {
    let machine = machine(FakeApi {
        locked: true,
        ..FakeApi::default()
    });
    assert_eq!(
        machine.setup.enroll(request(PASSWORD)).unwrap_err(),
        SetupError::Locked {
            retry_after: Some(900)
        }
    );
    nothing_left(&machine);
}

#[test]
fn a_credential_oxy_does_not_know_is_refused_before_the_password_is_sent() {
    let machine = machine(FakeApi::default());
    let mut request = request(PASSWORD);
    request.credential.device_secret = Secret::new("guessed");
    assert_eq!(
        machine.setup.enroll(request).unwrap_err(),
        SetupError::CredentialRefused
    );
    assert_eq!(machine.api().calls(), vec![Call::Mint]);
    nothing_left(&machine);
}

#[test]
fn the_account_must_be_the_one_the_credential_is_for() {
    // Not signed in on that device.
    let machine = machine(FakeApi {
        on_device: vec![OTHER_ACCOUNT],
        ..FakeApi::default()
    });
    assert_eq!(
        machine.setup.enroll(request(PASSWORD)).unwrap_err(),
        SetupError::AccountNotOnDevice
    );
    assert_eq!(machine.api().calls(), vec![Call::Mint]);
    nothing_left(&machine);

    // The token says it is another account's.
    let machine = machine_with(
        FakeApi {
            profile_id: Some(OTHER_ACCOUNT),
            ..FakeApi::default()
        },
        FakeUsers::default(),
    );
    assert_eq!(
        machine.setup.enroll(request(PASSWORD)).unwrap_err(),
        SetupError::AccountMismatch
    );
    assert_eq!(machine.api().calls(), vec![Call::Mint, Call::Profile]);
    nothing_left(&machine);

    // The password signed in another account, or on another device.
    for signs_in in [(OTHER_ACCOUNT, DEVICE), (ACCOUNT, "device-2")] {
        let machine = machine_with(
            FakeApi {
                signs_in: Some(signs_in),
                ..FakeApi::default()
            },
            FakeUsers::default(),
        );
        assert_eq!(
            machine.setup.enroll(request(PASSWORD)).unwrap_err(),
            SetupError::AccountMismatch
        );
        nothing_left(&machine);
    }
}

#[test]
fn an_authenticator_is_asked_for_and_a_wrong_code_may_be_followed_by_the_right_one() {
    let machine = machine(FakeApi {
        authenticator: true,
        ..FakeApi::default()
    });
    let Ok(Progress::SecondFactor(pending)) = machine.setup.enroll(request(PASSWORD)) else {
        panic!("a second factor is asked for");
    };
    // Nothing on the machine yet.
    nothing_left(&machine);
    assert_eq!(
        machine
            .setup
            .second_factor(&pending, &Secret::new("000000"))
            .unwrap_err(),
        SetupError::SecondFactorInvalid
    );
    nothing_left(&machine);
    let enrolled = machine
        .setup
        .second_factor(&pending, &Secret::new(CODE))
        .unwrap();
    assert_eq!(enrolled.account, account());
    assert_eq!(
        machine.setup.users().verifier(&enrolled.user).as_deref(),
        Some(PASSWORD)
    );
    assert_eq!(
        machine.api().calls(),
        vec![
            Call::Mint,
            Call::Profile,
            Call::Password,
            Call::SecondFactor,
            Call::SecondFactor
        ]
    );
}

#[test]
fn a_verifier_that_cannot_be_set_undoes_the_user_and_its_file() {
    let users = FakeUsers::default();
    users.fail(UserStep::SetVerifier);
    let machine = machine_with(FakeApi::default(), users);
    assert!(matches!(
        machine.setup.enroll(request(PASSWORD)),
        Err(SetupError::Failed(_))
    ));
    nothing_left(&machine);
    assert!(machine.setup.users().steps().contains(&UserStep::Remove));
}

/// Another account enrolled on the machine, written as a setup writes it, its user `uid`'s.
fn enrolled_other(machine: &Machine<FakeUsers>, id: &str, uid: u32) -> UserName {
    let account = AccountId::parse(id).unwrap();
    let user = account.user_name();
    machine.setup.users().add(user.as_str(), uid);
    machine
        .setup
        .store()
        .put(&crate::store::Enrollment {
            account: account.clone(),
            user: user.clone(),
            uid,
            profile: Profile {
                id: account.as_str().into(),
                username: None,
                display_name: None,
                avatar: None,
            },
            credential: None,
            password_changed: false,
        })
        .unwrap();
    user
}

#[test]
fn the_first_account_set_up_on_a_machine_is_its_administrator() {
    let machine = machine(FakeApi::default());
    let ada = enrolled(machine.setup.enroll(request(PASSWORD)));
    assert!(machine.setup.users().is_admin(ada.user.as_str()));
    // Made one before its verifier, which is set last.
    let steps = machine.setup.users().steps();
    let made = steps
        .iter()
        .position(|step| *step == UserStep::MakeAdministrator)
        .unwrap();
    assert!(made < steps.len() - 1);
    // Set up again, it stays one, made only once.
    let mut again = request(PASSWORD);
    again.credential.device_secret = Secret::new(OTHER_DEVICE_SECRET);
    enrolled(machine.setup.enroll(again));
    assert!(machine.setup.users().is_admin(ada.user.as_str()));
    assert_eq!(
        machine
            .setup
            .users()
            .steps()
            .iter()
            .filter(|step| **step == UserStep::MakeAdministrator)
            .count(),
        1
    );
}

#[test]
fn an_account_set_up_where_another_administers_the_machine_is_not_one() {
    let machine = machine(FakeApi::default());
    let other = enrolled_other(&machine, OTHER_ACCOUNT, 1500);
    machine
        .setup
        .users()
        .administrators
        .lock()
        .unwrap()
        .insert(other.as_str().into());
    let enrolled = enrolled(machine.setup.enroll(request(PASSWORD)));
    assert!(!machine.setup.users().is_admin(enrolled.user.as_str()));
    assert!(!machine
        .setup
        .users()
        .steps()
        .contains(&UserStep::MakeAdministrator));
}

#[test]
fn a_first_account_that_cannot_be_made_the_administrator_is_not_set_up() {
    let users = FakeUsers::default();
    users.fail(UserStep::MakeAdministrator);
    let machine = machine_with(FakeApi::default(), users);
    assert!(matches!(
        machine.setup.enroll(request(PASSWORD)),
        Err(SetupError::Failed(_))
    ));
    nothing_left(&machine);
    // Its platform credential was ended online, as any failed setup's is.
    assert!(machine.api().calls().contains(&Call::SignOut));
}

#[test]
fn a_machine_without_an_administrator_makes_its_first_account_one() {
    let machine = machine(FakeApi::default());
    // No account: nothing to do.
    assert_eq!(machine.setup.ensure_administrator(), Ok(None));
    let later = enrolled_other(&machine, OTHER_ACCOUNT, 1700);
    let first = enrolled_other(&machine, ACCOUNT, 1600);
    assert_eq!(
        machine.setup.ensure_administrator(),
        Ok(Some(first.clone()))
    );
    assert!(machine.setup.users().is_admin(first.as_str()));
    assert!(!machine.setup.users().is_admin(later.as_str()));
    // Once made, never again: an administrator removed makes no other account one.
    assert_eq!(machine.setup.ensure_administrator(), Ok(None));
    machine.setup.users().administrators.lock().unwrap().clear();
    assert_eq!(machine.setup.ensure_administrator(), Ok(None));
    assert!(!machine.setup.users().is_admin(later.as_str()));
}

#[test]
fn a_machine_whose_administrator_cannot_be_made_tries_again_at_its_next_start() {
    let machine = machine(FakeApi::default());
    let first = enrolled_other(&machine, ACCOUNT, 1600);
    machine.setup.users().fail(UserStep::MakeAdministrator);
    assert!(machine.setup.ensure_administrator().is_err());
    machine.setup.users().failing.lock().unwrap().clear();
    assert_eq!(machine.setup.ensure_administrator(), Ok(Some(first)));
}

#[test]
fn a_machine_that_had_its_administrator_makes_no_new_one_at_a_setup() {
    let machine = machine(FakeApi::default());
    let other = enrolled_other(&machine, OTHER_ACCOUNT, 1500);
    machine
        .setup
        .users()
        .administrators
        .lock()
        .unwrap()
        .insert(other.as_str().into());
    // The machine's administrator is seen, and recorded.
    assert_eq!(machine.setup.ensure_administrator(), Ok(None));
    // Then removed (not this service's concern here): the next account set up is no administrator.
    machine.setup.users().administrators.lock().unwrap().clear();
    let enrolled = enrolled(machine.setup.enroll(request(PASSWORD)));
    assert!(!machine.setup.users().is_admin(enrolled.user.as_str()));
}

#[test]
fn with_the_systems_tools_the_first_account_joins_the_administrators_group() {
    if !super::passwd_tools() {
        return;
    }
    let root = Root::new("setup-real-administrator");
    let machine = machine_with(FakeApi::default(), SystemUsers::with_prefix(root.path()));
    let enrolled = enrolled(machine.setup.enroll(request(PASSWORD)));
    assert!(root
        .read("group")
        .lines()
        .any(|line| line == format!("sudo:x:27:{}", enrolled.user)));
    assert!(machine
        .setup
        .users()
        .is_administrator(&enrolled.user)
        .unwrap());
}

#[test]
fn a_file_that_cannot_be_written_undoes_the_user() {
    let machine = machine(FakeApi::default());
    // A directory where the file's temporary would go: no write gets past it, even root's.
    std::fs::create_dir(
        machine
            .setup
            .store()
            .directory()
            .join(format!("{}.json.new", account().user_name())),
    )
    .unwrap();
    assert!(matches!(
        machine.setup.enroll(request(PASSWORD)),
        Err(SetupError::Failed(_))
    ));
    assert!(machine.setup.users().names().is_empty());
    assert!(machine.enrolled_accounts().is_empty());
    assert!(!machine
        .setup
        .users()
        .steps()
        .contains(&UserStep::SetVerifier));
}

#[test]
fn a_user_that_cannot_be_made_is_not_left_half_made() {
    let users = FakeUsers::default();
    users.fail(UserStep::Create);
    let machine = machine_with(FakeApi::default(), users);
    assert!(matches!(
        machine.setup.enroll(request(PASSWORD)),
        Err(SetupError::Failed(_))
    ));
    nothing_left(&machine);
    assert!(machine.setup.users().steps().contains(&UserStep::Remove));
}

#[test]
fn an_undo_that_fails_is_said() {
    let users = FakeUsers::default();
    users.fail(UserStep::SetVerifier);
    users.fail(UserStep::Remove);
    let machine = machine_with(FakeApi::default(), users);
    let error = machine.setup.enroll(request(PASSWORD)).unwrap_err();
    assert!(
        matches!(error, SetupError::RollbackFailed { .. }),
        "{error:?}"
    );
    // The user that is left was never given a verifier: its password is locked.
    let name = account().user_name();
    assert_eq!(machine.setup.users().verifier(&name), None);
    // Its file was forgotten all the same.
    assert_eq!(machine.files(), 0);
}

#[test]
fn an_enrolled_account_set_up_again_keeps_its_user_and_takes_the_new_credential() {
    let machine = machine(FakeApi::default());
    let first = enrolled(machine.setup.enroll(request(PASSWORD)));
    let mut again = request(PASSWORD);
    again.credential.device_secret = Secret::new(OTHER_DEVICE_SECRET);
    let second = enrolled(machine.setup.enroll(again));
    assert_eq!(first.uid, second.uid);
    let stored = machine.setup.store().get(&account()).unwrap().unwrap();
    assert_eq!(
        stored.credential.unwrap().device_secret.expose(),
        platform_secret(2)
    );
    assert_eq!(machine.setup.users().names().len(), 1);
    assert_eq!(
        machine
            .setup
            .users()
            .steps()
            .iter()
            .filter(|step| **step == UserStep::Create)
            .count(),
        1
    );
}

#[test]
fn an_enrolled_accounts_failed_update_puts_its_file_back() {
    let machine = machine(FakeApi::default());
    enrolled(machine.setup.enroll(request(PASSWORD)));
    let before = machine.setup.store().get(&account()).unwrap().unwrap();
    machine.setup.users().fail(UserStep::SetVerifier);
    let mut again = request(PASSWORD);
    again.credential.device_secret = Secret::new(OTHER_DEVICE_SECRET);
    assert!(matches!(
        machine.setup.enroll(again),
        Err(SetupError::Failed(_))
    ));
    assert_eq!(machine.setup.store().get(&account()).unwrap(), Some(before));
    assert_eq!(
        machine
            .setup
            .users()
            .verifier(&account().user_name())
            .as_deref(),
        Some(PASSWORD)
    );
}

#[test]
fn a_user_the_service_did_not_make_is_never_taken_over() {
    let machine = machine(FakeApi::default());
    // Whoever made it, and with whatever uid (0 among them), it is not an enrolled account's.
    machine.setup.users().add(account().user_name().as_str(), 0);
    assert!(matches!(
        machine.setup.enroll(request(PASSWORD)),
        Err(SetupError::Conflict(_))
    ));
    // Found out before Oxy is asked: no password attempt is spent.
    assert!(machine.api().calls().is_empty());
    assert_eq!(machine.setup.users().verifier(&account().user_name()), None);
    assert_eq!(machine.files(), 0);
}

#[test]
fn an_enrolled_account_whose_user_changed_is_not_set_up() {
    let machine = machine(FakeApi::default());
    let first = enrolled(machine.setup.enroll(request(PASSWORD)));
    let users = machine.setup.users();
    users.users.lock().unwrap().clear();
    assert!(matches!(
        machine.setup.enroll(request(PASSWORD)),
        Err(SetupError::Conflict(_))
    ));
    users.add(first.user.as_str(), first.uid + 7);
    assert!(matches!(
        machine.setup.enroll(request(PASSWORD)),
        Err(SetupError::Conflict(_))
    ));
}

#[test]
fn unusable_requests_never_reach_oxy() {
    for (account, password) in [
        ("not-an-id", PASSWORD),
        (ACCOUNT, ""),
        (ACCOUNT, "two\nlines"),
        (ACCOUNT, &"x".repeat(513)),
    ] {
        let machine = machine(FakeApi::default());
        let error = machine
            .setup
            .enroll(Request {
                credential: credential(),
                account: account.into(),
                password: Some(Secret::new(password)),
                pin: None,
            })
            .unwrap_err();
        assert!(
            matches!(
                error,
                SetupError::InvalidAccount | SetupError::UnusablePassword
            ),
            "{error:?}"
        );
        assert!(machine.api().calls().is_empty());
        nothing_left(&machine);
    }
}

#[test]
fn no_secret_is_printed() {
    let printed = format!("{:?}", request(PASSWORD));
    assert!(
        !printed.contains(PASSWORD) && !printed.contains(DEVICE_SECRET),
        "{printed}"
    );
    let machine = machine(FakeApi {
        authenticator: true,
        ..FakeApi::default()
    });
    let progress = machine.setup.enroll(request(PASSWORD));
    let printed = format!("{progress:?}");
    assert!(printed.contains("SecondFactor"));
    for secret in [PASSWORD, DEVICE_SECRET, CHALLENGE] {
        assert!(!printed.contains(secret), "{printed}");
    }
    // Every refusal's words.
    for error in [
        SetupError::Offline,
        SetupError::WrongPassword,
        SetupError::Locked { retry_after: None },
        SetupError::Failed("x".into()),
    ] {
        assert!(!error.to_string().contains(PASSWORD));
    }
}

#[test]
fn with_the_systems_tools_a_setup_makes_a_real_user_and_verifier() {
    if !super::passwd_tools() {
        return;
    }
    let root = Root::new("setup-real");
    let machine = machine_with(FakeApi::default(), SystemUsers::with_prefix(root.path()));
    let enrolled = enrolled(machine.setup.enroll(request(PASSWORD)));
    let name = enrolled.user.as_str();
    assert!(root
        .read("passwd")
        .contains(&format!("{name}:x:{}:", enrolled.uid)));
    let verifier = root.shadow(name).unwrap();
    assert!(verifier.starts_with("$y$"));
    assert!(crypt_verifies(PASSWORD, &verifier));
    assert!(root.path().join("home").join(name).is_dir());
}

#[test]
fn with_the_systems_tools_an_offline_setup_changes_no_file() {
    let root = Root::new("setup-real-offline");
    let before: Vec<String> = ["passwd", "shadow", "group", "gshadow"]
        .iter()
        .map(|file| root.read(file))
        .collect();
    let machine = machine_with(
        FakeApi {
            offline_from: Some(Call::Mint),
            ..FakeApi::default()
        },
        SystemUsers::with_prefix(root.path()),
    );
    assert_eq!(
        machine.setup.enroll(request(PASSWORD)).unwrap_err(),
        SetupError::Offline
    );
    let after: Vec<String> = ["passwd", "shadow", "group", "gshadow"]
        .iter()
        .map(|file| root.read(file))
        .collect();
    assert_eq!(before, after);
    assert_eq!(
        std::fs::read_dir(root.path().join("home")).unwrap().count(),
        0
    );
    assert_eq!(machine.files(), 0);
}

#[test]
fn with_the_systems_tools_a_failed_setup_removes_the_user_and_home() {
    if !super::passwd_tools() {
        return;
    }
    let root = Root::new("setup-real-undo");
    let mut users = SystemUsers::with_prefix(root.path());
    // A chpasswd that fails, as one would on a full disk.
    users.chpasswd = "/bin/false".into();
    let machine = machine_with(FakeApi::default(), users);
    assert!(matches!(
        machine.setup.enroll(request(PASSWORD)),
        Err(SetupError::Failed(_))
    ));
    let name = account().user_name();
    assert!(!root.read("passwd").contains(name.as_str()));
    assert!(!root.read("group").contains(name.as_str()));
    assert!(!root.path().join("home").join(name.as_str()).exists());
    assert_eq!(machine.files(), 0);
}

#[test]
fn without_oxyoss_client_id_nothing_is_set_up() {
    let dir = TempDir::new("setup-unconfigured");
    let setup = Setup::new(
        FakeApi::default(),
        FakeUsers::default(),
        Store::open(&dir.path().join("accounts")).unwrap(),
    );
    assert_eq!(
        setup.enroll(request(PASSWORD)).unwrap_err(),
        SetupError::NotConfigured
    );
    assert!(setup.api().calls().is_empty());
    assert!(setup.users().names().is_empty());
}

#[test]
fn a_platform_credential_the_machine_could_not_take_is_ended_online() {
    let machine = machine(FakeApi::default());
    machine.setup.users().fail(UserStep::SetVerifier);
    assert!(matches!(
        machine.setup.enroll(request(PASSWORD)),
        Err(SetupError::Failed(_))
    ));
    nothing_left(&machine);
    // Its device was signed out at Oxy with the credential the sign-in issued.
    assert_eq!(
        machine.api().calls(),
        vec![
            Call::Mint,
            Call::Profile,
            Call::Password,
            Call::Mint,
            Call::SignOut
        ]
    );
    assert!(machine.api().revoked.load(Ordering::SeqCst));
}

#[test]
fn the_second_factor_names_the_platform_too() {
    let machine = machine(FakeApi {
        authenticator: true,
        ..FakeApi::default()
    });
    let Ok(Progress::SecondFactor(pending)) = machine.setup.enroll(request(PASSWORD)) else {
        panic!("a second factor");
    };
    machine
        .setup
        .second_factor(&pending, &Secret::new(CODE))
        .unwrap();
    assert_eq!(
        *machine.api().platforms.lock().unwrap(),
        [Some(platform_client()), Some(platform_client())]
    );
    let stored = machine.setup.store().get(&account()).unwrap().unwrap();
    assert_eq!(
        stored.credential.unwrap().device_secret.expose(),
        platform_secret(1)
    );
}
