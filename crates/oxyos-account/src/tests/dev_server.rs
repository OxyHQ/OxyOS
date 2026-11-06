//! Setup against a real Oxy API: a development server on this machine with a database of its own,
//! never a production one. Ignored by default; run with
//!
//! ```text
//! OXYOS_ACCOUNT_DEV_API=http://127.0.0.1:55490 OXYOS_ACCOUNT_DEV_CLIENT_ID=… \
//! OXYOS_ACCOUNT_DEV_IDENTIFIER=… OXYOS_ACCOUNT_DEV_PASSWORD=… \
//! cargo test -p oxyos-account dev_server -- --ignored --test-threads 1
//! ```
//!
//! `OXYOS_ACCOUNT_DEV_CLIENT_ID` is the public client id of the development database's `OxyOS`
//! application (`platformHost: 'oxyos'`), whose registry recognises `so.oxy.dropsea`. The platform
//! tests (`dev_server_platform_*`) take an account of their own each, as the setup tests do:
//! `OXYOS_ACCOUNT_DEV_PLATFORM_IDENTIFIER` and `…_PLATFORM_PASSWORD` (they set it up, mint, rotate,
//! remove; each run needs the account fresh or signed out).
//!
//! An account with an authenticator is set up too when `OXYOS_ACCOUNT_DEV_TOTP_IDENTIFIER`,
//! `OXYOS_ACCOUNT_DEV_TOTP_PASSWORD` and `OXYOS_ACCOUNT_DEV_TOTP_COMMAND` (a shell command that
//! prints its current code) are given. Each account is a fresh one: a wrong password is tried
//! once, and an authenticator's code works once.
//!
//! The device credential is what the SDK's own sign-in obtains: a password sign-in without a
//! device's proof, which gives a new device and its holder secret. The users are made with the
//! system's tools on a root of the test's own, so nothing on this machine changes.

use super::{crypt_verifies, Root, TempDir};
use crate::api::{ApiError, Attestation, DeviceCredential, HttpApi, OxyApi, PlatformClient};
use crate::platform::{Checked, Local, Platform, Status, TokenReason};
use crate::setup::{Progress, Request, Setup, SetupError};
use crate::store::Store;
use crate::users::SystemUsers;
use crate::{AccountId, Secret};
use std::os::unix::fs::MetadataExt;

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// A JSON request to the development server, and its JSON answer.
fn post(url: String, body: serde_json::Value) -> serde_json::Value {
    let mut response = ureq::post(url)
        .header("Content-Type", "application/json")
        .send(serde_json::to_vec(&body).unwrap())
        .expect("the development server answers");
    serde_json::from_slice(&response.body_mut().read_to_vec().unwrap()).unwrap()
}

/// The SDK's sign-in, as the setup client runs it: the account's id and the device credential.
fn sdk_sign_in(api: &str, identifier: &str, password: &str) -> (String, DeviceCredential) {
    let body = post(
        format!("{api}/auth/signin/password"),
        serde_json::json!({"identifier": identifier, "password": password}),
    );
    let text = |value: &serde_json::Value| value.as_str().unwrap().to_owned();
    (
        text(&body["user"]["id"]),
        DeviceCredential {
            device_id: text(&body["deviceId"]),
            device_secret: Secret::new(text(&body["deviceSecret"])),
        },
    )
}

struct Machine {
    setup: Setup<HttpApi, SystemUsers>,
    root: Root,
    _state: TempDir,
}

fn client() -> PlatformClient {
    PlatformClient {
        client_id: env("OXYOS_ACCOUNT_DEV_CLIENT_ID").expect("OXYOS_ACCOUNT_DEV_CLIENT_ID"),
    }
}

fn machine(api: &str) -> Machine {
    let root = Root::new("dev-server");
    let state = TempDir::new("dev-server-state");
    let store = Store::open(&state.path().join("accounts")).unwrap();
    Machine {
        setup: Setup::new(
            HttpApi::new(api).unwrap(),
            SystemUsers::with_prefix(root.path()),
            store,
        )
        .with_platform(client()),
        root,
        _state: state,
    }
}

fn request(account: &str, credential: &DeviceCredential, password: &str) -> Request {
    Request {
        credential: DeviceCredential {
            device_id: credential.device_id.clone(),
            device_secret: Secret::new(credential.device_secret.expose()),
        },
        account: account.into(),
        password: Secret::new(password),
    }
}

fn assert_enrolled(
    machine: &Machine,
    account: &str,
    credential: &DeviceCredential,
    password: &str,
) {
    let account = AccountId::parse(account).unwrap();
    let name = account.user_name();
    let verifier = machine.root.shadow(name.as_str()).unwrap();
    assert!(verifier.starts_with("$y$"));
    assert!(crypt_verifies(password, &verifier));
    let stored = machine.setup.store().get(&account).unwrap().unwrap();
    // The platform credential the sign-in issued is kept: not the one handed over, which Oxy
    // retired as it issued it (R1).
    let kept = stored.credential.as_ref().unwrap();
    assert_eq!(kept.device_id, credential.device_id);
    assert_ne!(kept.device_secret, credential.device_secret);
    assert_eq!(
        machine
            .setup
            .api()
            .mint_token(credential, &account)
            .unwrap_err(),
        ApiError::InvalidDeviceSecret
    );
    let file = machine
        .setup
        .store()
        .directory()
        .join(format!("{name}.json"));
    assert_eq!(std::fs::metadata(file).unwrap().mode() & 0o777, 0o600);
    // The credential kept still mints for the account.
    machine
        .setup
        .api()
        .mint_token(stored.credential.as_ref().unwrap(), &account)
        .unwrap();
}

#[test]
#[ignore = "needs an Oxy API development server: see this file's documentation"]
fn dev_server_setup_with_a_password() {
    assert!(
        super::passwd_tools(),
        "the dev-server tests make users with the passwd tools"
    );
    let api = env("OXYOS_ACCOUNT_DEV_API").expect("OXYOS_ACCOUNT_DEV_API");
    let identifier = env("OXYOS_ACCOUNT_DEV_IDENTIFIER").expect("OXYOS_ACCOUNT_DEV_IDENTIFIER");
    let password = env("OXYOS_ACCOUNT_DEV_PASSWORD").expect("OXYOS_ACCOUNT_DEV_PASSWORD");
    let (account, credential) = sdk_sign_in(&api, &identifier, &password);
    let machine = machine(&api);
    let passwd = machine.root.read("passwd");

    // A wrong password: refused, nothing on the machine.
    let wrong = format!("{password}-not");
    assert_eq!(
        machine
            .setup
            .enroll(request(&account, &credential, &wrong))
            .unwrap_err(),
        SetupError::WrongPassword
    );
    assert_eq!(machine.root.read("passwd"), passwd);
    assert!(machine.setup.store().users().unwrap().is_empty());

    // Another account's id with this credential: not on the device.
    assert_eq!(
        machine
            .setup
            .enroll(request(
                "0190f6c4-8a1e-7b3c-9d2e-4f5a6b7c8d9e",
                &credential,
                &password
            ))
            .unwrap_err(),
        SetupError::AccountNotOnDevice
    );

    // The right one: the user, its verifier and the credential.
    let progress = machine
        .setup
        .enroll(request(&account, &credential, &password))
        .unwrap();
    let Progress::Enrolled(enrolled) = progress else {
        panic!("enrolled without a second factor: {progress:?}");
    };
    assert_eq!(enrolled.account.as_str(), account);
    assert_eq!(
        enrolled.profile.username.as_deref(),
        Some(identifier.as_str())
    );
    assert_enrolled(&machine, &account, &credential, &password);
}

#[test]
#[ignore = "needs an Oxy API development server: see this file's documentation"]
fn dev_server_setup_with_an_authenticator() {
    assert!(
        super::passwd_tools(),
        "the dev-server tests make users with the passwd tools"
    );
    let api = env("OXYOS_ACCOUNT_DEV_API").expect("OXYOS_ACCOUNT_DEV_API");
    let (Some(identifier), Some(password), Some(command)) = (
        env("OXYOS_ACCOUNT_DEV_TOTP_IDENTIFIER"),
        env("OXYOS_ACCOUNT_DEV_TOTP_PASSWORD"),
        env("OXYOS_ACCOUNT_DEV_TOTP_COMMAND"),
    ) else {
        eprintln!("no account with an authenticator given: skipped");
        return;
    };
    let code = || {
        let output = std::process::Command::new("sh")
            .args(["-c", &command])
            .output()
            .unwrap();
        assert!(output.status.success());
        Secret::new(String::from_utf8(output.stdout).unwrap().trim())
    };
    // The SDK's own sign-in passes the second factor too.
    let step = post(
        format!("{api}/auth/signin/password"),
        serde_json::json!({"identifier": identifier, "password": password}),
    );
    let body = post(
        format!("{api}/auth/signin/second-factor"),
        serde_json::json!({"challengeId": step["challengeId"], "code": code().expose()}),
    );
    let account = body["user"]["id"].as_str().unwrap().to_owned();
    let credential = DeviceCredential {
        device_id: body["deviceId"].as_str().unwrap().into(),
        device_secret: Secret::new(body["deviceSecret"].as_str().unwrap()),
    };

    // An authenticator's code works once per period: wait for the next one.
    std::thread::sleep(std::time::Duration::from_secs(31));
    let machine = machine(&api);
    let progress = machine
        .setup
        .enroll(request(&account, &credential, &password))
        .unwrap();
    let Progress::SecondFactor(pending) = progress else {
        panic!("a second factor is asked for: {progress:?}");
    };
    assert!(machine.setup.store().users().unwrap().is_empty());
    assert_eq!(
        machine
            .setup
            .second_factor(&pending, &Secret::new("000000"))
            .unwrap_err(),
        SetupError::SecondFactorInvalid
    );
    machine.setup.second_factor(&pending, &code()).unwrap();
    assert_enrolled(&machine, &account, &credential, &password);
}

/// An account of its own set up on a machine of the test's own, as the platform holds it.
fn platform_machine(
    api: &str,
) -> (
    Platform<HttpApi, SystemUsers>,
    Root,
    TempDir,
    String,
    String,
) {
    assert!(
        super::passwd_tools(),
        "the dev-server tests make users with the passwd tools"
    );
    let identifier = env("OXYOS_ACCOUNT_DEV_PLATFORM_IDENTIFIER")
        .expect("OXYOS_ACCOUNT_DEV_PLATFORM_IDENTIFIER");
    let password =
        env("OXYOS_ACCOUNT_DEV_PLATFORM_PASSWORD").expect("OXYOS_ACCOUNT_DEV_PLATFORM_PASSWORD");
    let (account, credential) = sdk_sign_in(api, &identifier, &password);
    let Machine {
        setup,
        root,
        _state: state,
    } = machine(api);
    let Progress::Enrolled(_) = setup
        .enroll(request(&account, &credential, &password))
        .unwrap()
    else {
        panic!("the platform account has no authenticator");
    };
    (Platform::new(setup), root, state, account, password)
}

#[test]
#[ignore = "needs an Oxy API development server: see this file's documentation"]
fn dev_server_platform_tokens_rotation_revocation_and_removal() {
    let api = env("OXYOS_ACCOUNT_DEV_API").expect("OXYOS_ACCOUNT_DEV_API");
    let (platform, _root, _state, account, password) = platform_machine(&api);
    let platform = std::sync::Arc::new(platform);
    let id = AccountId::parse(&account).unwrap();
    let stored = || platform.setup().store().get(&id).unwrap().unwrap();
    let uid = stored().uid;
    let user = id.user_name();

    // Tokens: Dropsea's by its desktop entry id, OxyOS's own; cached, minted again when refused.
    let dropsea = Attestation::Entry("so.oxy.dropsea".into());
    let first = platform
        .token(uid, &dropsea, TokenReason::Boot)
        .expect("a token for so.oxy.dropsea");
    assert!(!first.access_token.is_empty());
    assert_eq!(
        platform
            .token(uid, &dropsea, TokenReason::Preflight)
            .unwrap(),
        first
    );
    let own = platform
        .token(uid, &Attestation::OwnPlatform, TokenReason::Boot)
        .expect("a token for OxyOS itself");
    assert_ne!(own.access_token, first.access_token);
    let fresh = platform
        .token(uid, &dropsea, TokenReason::Response401)
        .unwrap();
    assert_ne!(fresh.access_token, first.access_token);
    // An application the registry does not recognise gets none.
    assert!(platform
        .token(
            uid,
            &Attestation::Entry("so.oxy.not-an-application".into()),
            TokenReason::Boot
        )
        .is_none());
    let snapshot = platform.snapshot(uid);
    assert_eq!(snapshot.status, Status::SignedIn);
    assert!(snapshot.online);
    assert_eq!(snapshot.account.unwrap().id, account);

    // Every online check is the platform's own sign-in: the credential rotates, the old one is
    // retired, the new one mints.
    let before = stored().credential.unwrap();
    assert_eq!(
        platform.check(
            user.as_str(),
            Secret::new(password.as_str()),
            Local::Accepted
        ),
        Checked::ACCEPTED
    );
    platform.settle();
    let after = stored().credential.unwrap();
    assert_ne!(after.device_secret, before.device_secret);
    assert_eq!(
        platform.setup().api().mint_token(&before, &id).unwrap_err(),
        ApiError::InvalidDeviceSecret
    );
    assert!(platform
        .token(uid, &dropsea, TokenReason::Preflight)
        .is_some());
    // A wrong password, PAM rejecting it too: rejected, nothing rotated.
    assert_eq!(
        platform.check(
            user.as_str(),
            Secret::new("not the password"),
            Local::Rejected
        ),
        Checked::REJECTED
    );
    assert_eq!(stored().credential.unwrap(), after);

    // Removed: the device signed out at Oxy, the account forgotten.
    let enrollment = platform.remove(uid).unwrap();
    assert_eq!(
        platform.setup().api().mint_token(&after, &id).unwrap_err(),
        ApiError::AccountNotOnDevice
    );
    platform.finish_removal(&enrollment);
    assert!(platform.setup().store().users().unwrap().is_empty());
}

#[test]
#[ignore = "needs an Oxy API development server: see this file's documentation"]
fn dev_server_platform_revocation_signs_the_account_out() {
    let api = env("OXYOS_ACCOUNT_DEV_API").expect("OXYOS_ACCOUNT_DEV_API");
    let (platform, _root, _state, account, password) = platform_machine(&api);
    let platform = std::sync::Arc::new(platform);
    let id = AccountId::parse(&account).unwrap();
    let stored = platform.setup().store().get(&id).unwrap().unwrap();
    let credential = stored.credential.unwrap();
    // Oxy's own device list ends the device's session (as removing it there would).
    let token = platform.setup().api().mint_token(&credential, &id).unwrap();
    platform.setup().api().sign_out_device(&token).unwrap();
    // The next mint finds it gone: signed out on the machine.
    assert!(platform
        .token(stored.uid, &Attestation::OwnPlatform, TokenReason::Boot)
        .is_none());
    let now = platform.setup().store().get(&id).unwrap().unwrap();
    assert_eq!(now.state(), crate::store::State::SignInAgain);
    assert_eq!(platform.snapshot(stored.uid).status, Status::SignedOut);
    assert_eq!(
        platform
            .check(
                id.user_name().as_str(),
                Secret::new(password),
                Local::Accepted
            )
            .reason,
        Some(crate::platform::CheckReason::SignInAgain)
    );
}
