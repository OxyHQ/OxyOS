//! The machine's Oxy session once an account is set up: tokens and their cache, the snapshot, the
//! online checks, revocation, the rotation of the platform credential, and removing an account.
//! Oxy and the users are fakes; the store is real, in a directory of the test's own.

use super::*;
use crate::platform::{
    attestation, CheckReason, Checked, Local, Platform, Status, TokenReason, REFRESH_BEFORE,
};
use crate::setup::{Progress, Request, Setup};
use crate::store::{State, Store};
use std::sync::Arc;

pub(crate) type Service = Platform<FakeApi, FakeUsers>;

pub(crate) struct Machine {
    pub(crate) platform: Arc<Service>,
    pub(crate) dir: Arc<TempDir>,
}

impl Machine {
    pub(crate) fn api(&self) -> &FakeApi {
        self.platform.setup().api()
    }

    pub(crate) fn users(&self) -> &FakeUsers {
        self.platform.setup().users()
    }

    pub(crate) fn stored(&self) -> crate::store::Enrollment {
        self.platform
            .setup()
            .store()
            .get(&account())
            .unwrap()
            .unwrap()
    }

    pub(crate) fn uid(&self) -> u32 {
        self.stored().uid
    }

    /// The same machine, its Oxy another (offline, say).
    pub(crate) fn with_api(&self, api: FakeApi) -> Machine {
        let store = Store::open(&self.dir.path().join("accounts")).unwrap();
        let users = FakeUsers::default();
        for name in self.users().names() {
            users.add(&name, self.uid());
            if let Some(verifier) = self.users().verifier(&account().user_name()) {
                users.users.lock().unwrap().get_mut(&name).unwrap().1 = Some(verifier);
            }
        }
        *users.administrators.lock().unwrap() = self.users().administrators.lock().unwrap().clone();
        Machine {
            platform: Arc::new(Platform::new(
                Setup::new(api, users, store).with_platform(platform_client()),
            )),
            dir: self.dir.clone(),
        }
    }
}

/// A machine with Ada set up with `password` (which `api` must accept), its user's uid `uid`.
pub(crate) fn machine_as(api: FakeApi, password: &str, uid: Option<u32>) -> Machine {
    let dir = TempDir::new("platform");
    let store = Store::open(&dir.path().join("accounts")).unwrap();
    let setup = Setup::new(api, FakeUsers::default(), store).with_platform(platform_client());
    let request = Request {
        credential: credential(),
        account: ACCOUNT.into(),
        password: Some(Secret::new(password)),
        pin: None,
        appearance: None,
    };
    match setup.enroll(request).unwrap() {
        Progress::Enrolled(_) => {}
        Progress::SecondFactor(mut pending) => {
            setup
                .second_factor(&mut pending, &Secret::new(CODE))
                .unwrap();
        }
    }
    if let Some(uid) = uid {
        let mut stored = setup.store().get(&account()).unwrap().unwrap();
        stored.uid = uid;
        setup.store().put(&stored).unwrap();
        let name = account().user_name();
        let mut users = setup.users().users.lock().unwrap();
        users.get_mut(name.as_str()).unwrap().0.uid = uid;
    }
    setup.api().calls.lock().unwrap().clear();
    Machine {
        platform: Arc::new(Platform::new(setup)),
        dir: Arc::new(dir),
    }
}

pub(crate) fn machine(api: FakeApi) -> Machine {
    machine_as(api, PASSWORD, None)
}

fn dropsea() -> Attestation {
    Attestation::Entry("so.oxy.dropsea".into())
}

fn user() -> String {
    account().user_name().as_str().to_owned()
}

#[test]
fn settings_and_the_shell_as_system_applications_are_oxyos_itself() {
    assert_eq!(
        attestation("so.oxy.settings", true),
        Attestation::OwnPlatform
    );
    assert_eq!(attestation("so.oxy.shell", true), Attestation::OwnPlatform);
    // Not registered as system applications, they are what their ids say, as any other.
    assert_eq!(
        attestation("so.oxy.settings", false),
        Attestation::Entry("so.oxy.settings".into())
    );
    assert_eq!(attestation("so.oxy.dropsea", true), dropsea());
}

#[test]
fn a_token_is_cached_per_application_and_minted_again_when_refused() {
    let machine = machine(FakeApi::default());
    let uid = machine.uid();
    let first = machine
        .platform
        .token(uid, &dropsea(), TokenReason::Boot)
        .unwrap();
    assert_eq!(first.access_token.expose(), "app-token-1");
    // Cached: the same token, not minted again.
    let again = machine
        .platform
        .token(uid, &dropsea(), TokenReason::Preflight)
        .unwrap();
    assert_eq!(again, first);
    // Another application's is its own.
    let own = machine
        .platform
        .token(uid, &Attestation::OwnPlatform, TokenReason::Boot)
        .unwrap();
    assert_eq!(own.access_token.expose(), "app-token-2");
    // Refused by the API it was shown to: a fresh one at once.
    let fresh = machine
        .platform
        .token(uid, &dropsea(), TokenReason::Response401)
        .unwrap();
    assert_eq!(fresh.access_token.expose(), "app-token-3");
    assert_eq!(
        *machine.api().minted.lock().unwrap(),
        [dropsea(), Attestation::OwnPlatform, dropsea()]
    );
    // Its Debug never prints it.
    assert!(!format!("{fresh:?}").contains("app-token"));
}

#[test]
fn a_token_about_to_expire_is_minted_again() {
    let life = REFRESH_BEFORE.as_secs() as i64 - 10;
    let machine = machine(FakeApi {
        token_life: life,
        ..FakeApi::default()
    });
    let uid = machine.uid();
    for n in 1..=3 {
        let token = machine
            .platform
            .token(uid, &dropsea(), TokenReason::Preflight)
            .unwrap();
        assert_eq!(token.access_token.expose(), format!("app-token-{n}"));
    }
}

#[test]
fn offline_there_is_no_token_and_the_snapshot_still_says_signed_in() {
    let online = machine(FakeApi::default());
    let uid = online.uid();
    let machine = online.with_api(FakeApi {
        offline_from: Some(Call::Mint),
        ..FakeApi::default()
    });
    assert_eq!(
        machine.platform.token(uid, &dropsea(), TokenReason::Boot),
        None
    );
    let snapshot = machine.platform.snapshot(uid);
    assert_eq!(snapshot.status, Status::SignedIn);
    assert!(!snapshot.online);
    let account = snapshot.account.unwrap();
    assert_eq!(account.id, ACCOUNT);
    assert_eq!(account.handle, USERNAME);
    assert_eq!(account.display_name.as_deref(), Some("Ada Lovelace"));
}

#[test]
fn the_snapshot_is_the_sdks_and_its_revision_changes_only_with_its_account() {
    let machine = machine(FakeApi::default());
    let uid = machine.uid();
    let before = machine.platform.snapshot(uid);
    assert!(!before.online);
    machine.platform.token(uid, &dropsea(), TokenReason::Boot);
    // Oxy reached: told, with the same revision (the SDK takes a new one for another account).
    let after = machine.platform.snapshot(uid);
    assert!(after.online);
    assert_eq!(after.revision, before.revision);
    let json = serde_json::to_value(&after).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "status": "signed_in",
            "revision": after.revision,
            "account": {
                "id": ACCOUNT,
                "handle": USERNAME,
                "displayName": "Ada Lovelace",
                "avatar": "avatar-file-1",
            },
            "online": true,
        })
    );
    // A user with no account: signed out, no account.
    // Signed out by Oxy: a new revision.
    machine.api().revoke();
    machine
        .platform
        .token(uid, &Attestation::OwnPlatform, TokenReason::Boot);
    let signed_out = machine.platform.snapshot(uid);
    assert_eq!(signed_out.status, Status::SignedOut);
    assert!(signed_out.revision > after.revision);
    assert_eq!(machine.platform.snapshot(uid).revision, signed_out.revision);
    let nobody = machine.platform.snapshot(uid + 1);
    assert_eq!(nobody.status, Status::SignedOut);
    assert_eq!(nobody.account, None);
    assert_eq!(
        serde_json::to_value(&nobody).unwrap()["status"],
        "signed_out"
    );
}

#[test]
fn oxy_ending_the_device_session_signs_the_account_out_on_the_machine() {
    let machine = machine(FakeApi::default());
    let uid = machine.uid();
    let name = account().user_name();
    machine.api().revoke();
    assert_eq!(
        machine.platform.token(uid, &dropsea(), TokenReason::Boot),
        None
    );
    // Its credential forgotten, its verifier locked, its sessions ended; its user and home kept.
    let stored = machine.stored();
    assert_eq!(stored.credential, None);
    assert_eq!(stored.state(), State::SignInAgain);
    assert!(machine
        .users()
        .locked
        .lock()
        .unwrap()
        .contains(name.as_str()));
    assert_eq!(*machine.users().ended.lock().unwrap(), [name.as_str()]);
    assert_eq!(machine.users().names(), [name.as_str()]);
    let snapshot = machine.platform.snapshot(uid);
    assert_eq!(snapshot.status, Status::SignedOut);
    assert_eq!(snapshot.account, None);
    // It lists as signInAgain, and no password signs it in until it is set up again.
    let (listed, _) = machine.platform.setup().store().list().unwrap();
    assert_eq!(listed[0].state, State::SignInAgain);
    assert_eq!(
        machine
            .platform
            .check(&user(), Secret::new(PASSWORD), Local::Accepted),
        Checked {
            result: crate::platform::Verdict::Rejected,
            reason: Some(CheckReason::SignInAgain),
        }
    );
    // Set up again online: the same user, a credential anew.
    machine
        .api()
        .revoked
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let mut request = Request {
        credential: credential(),
        account: ACCOUNT.into(),
        password: Some(Secret::new(PASSWORD)),
        pin: None,
        appearance: None,
    };
    request.credential.device_secret = Secret::new(OTHER_DEVICE_SECRET);
    let Progress::Enrolled(enrolled) = machine.platform.setup().enroll(request).unwrap() else {
        panic!("enrolled");
    };
    assert_eq!(enrolled.uid, uid);
    assert_eq!(machine.stored().state(), State::Ready);
}

#[test]
fn a_password_pam_rejects_that_oxy_accepts_becomes_the_verifier() {
    // The password was changed elsewhere: Oxy knows only the new one; the machine the old.
    const NEW: &str = "a new password set elsewhere";
    let machine = machine_as(
        FakeApi {
            password: NEW,
            ..FakeApi::default()
        },
        NEW,
        None,
    );
    let name = account().user_name();
    machine
        .users()
        .users
        .lock()
        .unwrap()
        .get_mut(name.as_str())
        .unwrap()
        .1 = Some(PASSWORD.into());
    assert_eq!(
        machine
            .platform
            .check(&user(), Secret::new(NEW), Local::Rejected),
        Checked::ACCEPTED
    );
    assert_eq!(machine.users().verifier(&name).as_deref(), Some(NEW));
    // The platform's own sign-in, which rotated the credential, confirmed (R8).
    assert_eq!(
        machine.api().calls(),
        [Call::Mint, Call::Profile, Call::Password, Call::Confirm]
    );
    assert_eq!(
        machine.stored().credential.unwrap().device_secret.expose(),
        platform_secret(2)
    );
    assert_eq!(
        machine.api().platforms.lock().unwrap().last().unwrap(),
        &Some(platform_client())
    );
}

#[test]
fn a_wrong_password_or_no_network_is_rejected_and_changes_nothing() {
    let machine = machine(FakeApi::default());
    let name = account().user_name();
    let before = machine.stored();
    assert_eq!(
        machine
            .platform
            .check(&user(), Secret::new("not it"), Local::Rejected),
        Checked::REJECTED
    );
    assert_eq!(machine.users().verifier(&name).as_deref(), Some(PASSWORD));
    assert_eq!(machine.stored(), before);
    let offline = machine.with_api(FakeApi {
        offline_from: Some(Call::Mint),
        ..FakeApi::default()
    });
    assert_eq!(
        offline
            .platform
            .check(&user(), Secret::new("whatever"), Local::Rejected),
        Checked::REJECTED
    );
    // An account that is not enrolled.
    assert_eq!(
        machine
            .platform
            .check("oxy-nobody", Secret::new(PASSWORD), Local::Accepted),
        Checked::REJECTED
    );
}

#[test]
fn a_password_pam_accepts_is_accepted_at_once_and_oxys_refusal_flags_it() {
    const NEW: &str = "a new password set elsewhere";
    let machine = machine(FakeApi::default());
    let uid = machine.uid();
    // Oxy now knows another password (changed elsewhere), the machine the old one.
    let changed = machine.with_api(FakeApi {
        password: NEW,
        ..FakeApi::default()
    });
    // Rotation needs the live secret: the fake's live one is the one the machine holds.
    *changed.api().secrets.lock().unwrap() = vec![machine
        .stored()
        .credential
        .unwrap()
        .device_secret
        .expose()
        .into()];
    changed.api().issued.store(1, Ordering::SeqCst);
    assert_eq!(
        changed
            .platform
            .check(&user(), Secret::new(PASSWORD), Local::Accepted),
        Checked::ACCEPTED
    );
    changed.platform.settle();
    assert!(changed.stored().password_changed);
    // Flagged and online: the old password waits for Oxy, which refuses it.
    assert_eq!(
        changed
            .platform
            .check(&user(), Secret::new(PASSWORD), Local::Accepted),
        Checked {
            result: crate::platform::Verdict::Rejected,
            reason: Some(CheckReason::PasswordChanged),
        }
    );
    // Flagged and offline: the old password still opens it.
    let offline = changed.with_api(FakeApi {
        offline_from: Some(Call::Mint),
        ..FakeApi::default()
    });
    assert_eq!(
        offline
            .platform
            .check(&user(), Secret::new(PASSWORD), Local::Accepted),
        Checked::ACCEPTED
    );
    // The new one, which PAM does not know yet: Oxy accepts it, it becomes the verifier and the
    // flag is cleared.
    assert_eq!(
        changed
            .platform
            .check(&user(), Secret::new(NEW), Local::Rejected),
        Checked::ACCEPTED
    );
    assert!(!changed.stored().password_changed);
    assert_eq!(
        changed.users().verifier(&account().user_name()).as_deref(),
        Some(NEW)
    );
    assert_eq!(changed.platform.snapshot(uid).status, Status::SignedIn);
}

#[test]
fn every_online_sign_in_rotates_the_platform_credential() {
    let machine = machine(FakeApi::default());
    let uid = machine.uid();
    let token = machine
        .platform
        .token(uid, &dropsea(), TokenReason::Boot)
        .unwrap();
    for n in 2..=4 {
        assert_eq!(
            machine
                .platform
                .check(&user(), Secret::new(PASSWORD), Local::Accepted),
            Checked::ACCEPTED
        );
        machine.platform.settle();
        let stored = machine.stored();
        assert_eq!(
            stored.credential.unwrap().device_secret.expose(),
            platform_secret(n)
        );
        // The one before is retired at Oxy.
        let live = machine.api().secrets.lock().unwrap().clone();
        assert!(live.contains(&platform_secret(n)));
        assert!(!live.contains(&platform_secret(n - 1)));
    }
    // Written whole: nothing half-written beside the file (and the machine's administrator made,
    // a fact of its own, and its local sign-in).
    let mut files: Vec<String> = std::fs::read_dir(machine.platform.setup().store().directory())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(
        files,
        [
            crate::store::ADMINISTRATOR_MADE.to_owned(),
            format!("{}.json", user()),
            format!("{}.local", user())
        ]
    );
    // The old session's tokens are not handed out again.
    let after = machine
        .platform
        .token(uid, &dropsea(), TokenReason::Preflight)
        .unwrap();
    assert_ne!(after, token);
}

#[test]
fn an_authenticators_challenge_never_makes_a_new_password_the_verifier() {
    const NEW: &str = "a new password set elsewhere";
    let machine = machine_as(
        FakeApi {
            authenticator: true,
            password: NEW,
            ..FakeApi::default()
        },
        NEW,
        None,
    );
    let name = account().user_name();
    machine
        .users()
        .users
        .lock()
        .unwrap()
        .get_mut(name.as_str())
        .unwrap()
        .1 = Some(PASSWORD.into());
    let before = machine.stored().credential;
    // PAM rejects the new one; Oxy proves its first factor and asks the authenticator's code,
    // which a check cannot answer: rejected, the verifier kept (setting up again asks the code).
    assert_eq!(
        machine
            .platform
            .check(&user(), Secret::new(NEW), Local::Rejected),
        Checked {
            result: crate::platform::Verdict::Rejected,
            reason: Some(CheckReason::PasswordChanged),
        }
    );
    assert_eq!(machine.users().verifier(&name).as_deref(), Some(PASSWORD));
    assert_eq!(machine.stored().credential, before);
    // The one the machine knows, PAM accepting it, stays accepted.
    machine
        .users()
        .users
        .lock()
        .unwrap()
        .get_mut(name.as_str())
        .unwrap()
        .1 = Some(NEW.into());
    assert_eq!(
        machine
            .platform
            .check(&user(), Secret::new(NEW), Local::Accepted),
        Checked::ACCEPTED
    );
    machine.platform.settle();
    assert_eq!(machine.stored().credential, before);
}

#[test]
fn removing_an_account_ends_its_device_online_and_forgets_it() {
    let machine = machine(FakeApi::default());
    let uid = machine.uid();
    let name = account().user_name();
    let enrollment = machine.platform.remove(uid).unwrap();
    // Online: its device signed out with its own token; at once its verifier locked and its
    // credential forgotten.
    assert_eq!(machine.api().calls(), [Call::Mint, Call::SignOut]);
    assert!(machine.api().revoked.load(Ordering::SeqCst));
    assert!(machine
        .users()
        .locked
        .lock()
        .unwrap()
        .contains(name.as_str()));
    assert_eq!(machine.stored().credential, None);
    assert_eq!(machine.platform.snapshot(uid).status, Status::SignedOut);
    // Then its sessions, its user and home, its file.
    machine.platform.finish_removal(&enrollment);
    assert_eq!(*machine.users().ended.lock().unwrap(), [name.as_str()]);
    assert!(machine.users().names().is_empty());
    assert!(machine.platform.setup().store().users().unwrap().is_empty());
    assert_eq!(
        machine.platform.remove(uid).unwrap_err(),
        crate::platform::RemoveError::NotEnrolled
    );
}

#[test]
fn removing_an_account_offline_still_forgets_it() {
    let online = machine(FakeApi::default());
    let uid = online.uid();
    let machine = online.with_api(FakeApi {
        offline_from: Some(Call::Mint),
        ..FakeApi::default()
    });
    let enrollment = machine.platform.remove(uid).unwrap();
    assert!(!machine.api().calls().contains(&Call::SignOut));
    machine.platform.finish_removal(&enrollment);
    assert!(machine.platform.setup().store().users().unwrap().is_empty());
}

#[test]
fn a_user_that_cannot_be_removed_stays_signed_out() {
    let machine = machine(FakeApi::default());
    let uid = machine.uid();
    machine.users().fail(UserStep::Remove);
    let enrollment = machine.platform.remove(uid).unwrap();
    // userdel keeps failing (its processes, say): after the wait, the file stays, signed out.
    machine.platform.finish_removal(&enrollment);
    assert_eq!(machine.stored().state(), State::SignInAgain);
    assert_eq!(machine.users().names(), [account().user_name().as_str()]);
}

#[test]
fn mints_are_rationed_per_application() {
    let machine = machine(FakeApi::default());
    let uid = machine.uid();
    // An application Oxy does not recognise: refused once, then not asked again for a while.
    let unknown = Attestation::Entry("org.example.unknown".into());
    assert_eq!(
        machine.platform.token(uid, &unknown, TokenReason::Boot),
        None
    );
    assert_eq!(
        machine
            .platform
            .token(uid, &unknown, TokenReason::Response401),
        None
    );
    let asked = |api: &FakeApi| {
        api.calls()
            .iter()
            .filter(|call| **call == Call::AppToken)
            .count()
    };
    assert_eq!(asked(machine.api()), 1);
    // However often an application says its token was refused, at most MAX_MINTS in the window.
    for _ in 0..crate::platform::MAX_MINTS + 5 {
        machine
            .platform
            .token(uid, &dropsea(), TokenReason::Response401);
    }
    assert_eq!(asked(machine.api()), 1 + crate::platform::MAX_MINTS);
    // The cached token is still handed out meanwhile; another application has its own ration.
    assert!(machine
        .platform
        .token(uid, &dropsea(), TokenReason::Preflight)
        .is_some());
    assert!(machine
        .platform
        .token(uid, &Attestation::OwnPlatform, TokenReason::Boot)
        .is_some());
}

#[test]
fn a_rotation_oxy_could_not_confirm_keeps_the_credential_the_machine_had() {
    const NEW: &str = "a new password set elsewhere";
    let machine = machine_as(
        FakeApi {
            password: NEW,
            ..FakeApi::default()
        },
        NEW,
        None,
    );
    let before = machine.stored().credential.unwrap().device_secret;
    machine
        .api()
        .confirm_hiccups
        .lock()
        .unwrap()
        .push_back(Hiccup::Unreachable);
    // The password was changed elsewhere: Oxy accepts the new one, which becomes the verifier,
    // and the rotation waits for another check.
    let name = account().user_name();
    machine
        .users()
        .users
        .lock()
        .unwrap()
        .get_mut(name.as_str())
        .unwrap()
        .1 = Some(PASSWORD.into());
    assert_eq!(
        machine
            .platform
            .check(&user(), Secret::new(NEW), Local::Rejected),
        Checked::ACCEPTED
    );
    assert_eq!(machine.users().verifier(&name).as_deref(), Some(NEW));
    assert_eq!(machine.stored().credential.unwrap().device_secret, before);
    // The credential the machine has still works: the rotated one was never confirmed (R8).
    assert!(machine.api().live().contains(&before.expose().to_owned()));
    assert!(machine.api().is_pending(&platform_secret(2)));
}
