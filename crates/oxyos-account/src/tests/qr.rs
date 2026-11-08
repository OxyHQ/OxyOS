//! A setup by the QR whose proof is spent once, and only for a setup that can finish (2026-11-08,
//! a real machine: the proof redeemed, then `useradd` failed, and the retry found it spent).
//! Whatever fails for a moment is tried again without a second scan: the machine prepared before
//! the proof is spent, the redemption asked again with its key (Oxy ADR 0058 R7), and a platform
//! credential the machine could not take kept for the retry, ended online only when the setup ends
//! without success.

use super::*;
use crate::setup::{Appearance, Attempt, Event, Request, Setup, SetupError, Stage, Step};
use crate::store::Store;
use std::time::Duration;

const PIN: &str = "482916";

fn request() -> Request {
    Request {
        credential: credential(),
        account: ACCOUNT.into(),
        password: None,
        pin: Some(Secret::new(PIN)),
        appearance: None,
    }
}

/// A machine whose redemptions are asked again at once, `retries` times.
fn setup(api: FakeApi, retries: usize) -> (Setup<FakeApi, FakeUsers>, TempDir) {
    let dir = TempDir::new("qr");
    let store = Store::open(&dir.path().join("accounts")).unwrap();
    let setup = Setup::new(api, FakeUsers::default(), store)
        .with_platform(platform_client())
        .with_retry_delays(vec![Duration::ZERO; retries])
        .with_sleep(|_| {});
    (setup, dir)
}

/// The setup's turn, and the steps it told.
fn advance(
    setup: &Setup<FakeApi, FakeUsers>,
    attempt: &mut Attempt,
) -> (Vec<Stage>, Result<Step, SetupError>) {
    let mut stages = Vec::new();
    let result = setup.advance(attempt, &mut |event| {
        if let Event::Started(stage) = event {
            stages.push(stage)
        }
    });
    (stages, result)
}

fn files(setup: &Setup<FakeApi, FakeUsers>) -> usize {
    std::fs::read_dir(setup.store().directory())
        .unwrap()
        .count()
}

fn stored_secret(setup: &Setup<FakeApi, FakeUsers>) -> String {
    let stored = setup.store().get(&account()).unwrap().unwrap();
    stored.credential.unwrap().device_secret.expose().to_owned()
}

fn platform_calls(setup: &Setup<FakeApi, FakeUsers>) -> usize {
    setup
        .api()
        .calls()
        .iter()
        .filter(|call| **call == Call::PlatformDevice)
        .count()
}

#[test]
fn a_qr_setup_tells_its_steps_in_order_and_spends_the_proof_last() {
    let (setup, _dir) = setup(FakeApi::default(), 2);
    let mut attempt = setup.start(request()).unwrap();
    let (stages, result) = advance(&setup, &mut attempt);
    assert!(matches!(result, Ok(Step::Enrolled(_))), "{result:?}");
    assert_eq!(
        stages,
        [
            Stage::Securing,
            Stage::Verifying,
            Stage::Preparing,
            Stage::SigningIn,
            Stage::Finishing,
            Stage::Confirming
        ]
    );
    // The user was made before the proof was spent (useradd is the redemption's precondition),
    // and never removed.
    assert!(setup.users().steps().contains(&UserStep::Create));
    assert!(!setup.users().steps().contains(&UserStep::Remove));
    assert_eq!(stored_secret(&setup), platform_secret(1));
    assert!(!attempt.holds_credential());
}

#[test]
fn a_user_that_cannot_be_made_leaves_the_proof_unspent_and_the_retry_needs_no_scan() {
    let (setup, _dir) = setup(FakeApi::default(), 2);
    setup.users().fail(UserStep::Create);
    let mut attempt = setup.start(request()).unwrap();
    let (_, result) = advance(&setup, &mut attempt);
    assert!(matches!(result, Err(SetupError::Failed(_))), "{result:?}");
    // Oxy was never asked to redeem: the setup client's credential is live, nothing is left.
    assert_eq!(platform_calls(&setup), 0);
    assert!(setup.api().live().contains(&DEVICE_SECRET.to_string()));
    assert!(setup.users().names().is_empty());
    assert_eq!(files(&setup), 0);
    // The home can be made now: the same setup, the same proof.
    setup.users().failing.lock().unwrap().clear();
    let (_, result) = advance(&setup, &mut attempt);
    assert!(matches!(result, Ok(Step::Enrolled(_))), "{result:?}");
    assert_eq!(platform_calls(&setup), 1);
    assert_eq!(stored_secret(&setup), platform_secret(1));
}

#[test]
fn a_redemption_that_fails_on_its_way_is_asked_again_with_the_same_key() {
    let (setup, _dir) = setup(FakeApi::default(), 2);
    setup
        .api()
        .hiccup(&[Hiccup::Unreachable, Hiccup::Unavailable]);
    let mut attempt = setup.start(request()).unwrap();
    let (stages, result) = advance(&setup, &mut attempt);
    assert!(matches!(result, Ok(Step::Enrolled(_))), "{result:?}");
    // Three asks, one key; one stage for all of them.
    let keys = setup.api().keys.lock().unwrap().clone();
    assert_eq!(keys.len(), 3);
    assert!(keys.iter().all(|key| *key == keys[0]));
    assert_eq!(
        stages
            .iter()
            .filter(|stage| **stage == Stage::SigningIn)
            .count(),
        1
    );
    assert_eq!(stored_secret(&setup), platform_secret(1));
}

#[test]
fn an_answer_lost_on_its_way_is_redeemed_again_and_the_first_credential_revoked() {
    let (setup, _dir) = setup(FakeApi::default(), 2);
    setup.api().hiccup(&[Hiccup::AnswerLost]);
    let mut attempt = setup.start(request()).unwrap();
    let (_, result) = advance(&setup, &mut attempt);
    assert!(matches!(result, Ok(Step::Enrolled(_))), "{result:?}");
    // Oxy redeemed twice with the one key: the credential whose answer was lost is revoked, the
    // one the machine keeps is the only platform credential live.
    assert_eq!(platform_calls(&setup), 2);
    assert_eq!(stored_secret(&setup), platform_secret(2));
    let live = setup.api().live();
    assert!(!live.contains(&platform_secret(1)));
    assert!(live.contains(&platform_secret(2)));
    assert!(!live.contains(&DEVICE_SECRET.to_string()));
}

#[test]
fn oxy_unreachable_after_every_ask_is_a_retry_not_a_rescan() {
    let (setup, _dir) = setup(FakeApi::default(), 2);
    setup
        .api()
        .hiccup(&[Hiccup::Unreachable, Hiccup::AnswerLost, Hiccup::Unavailable]);
    let mut attempt = setup.start(request()).unwrap();
    let (_, result) = advance(&setup, &mut attempt);
    assert_eq!(result.unwrap_err(), SetupError::Offline);
    // The user made for it is removed; the redemption Oxy made (its answer lost) is not the
    // machine's yet.
    assert!(setup.users().names().is_empty());
    assert_eq!(files(&setup), 0);
    assert!(!attempt.holds_credential());
    // The person tries again: the redemption repeated with the same key, no scan.
    let (_, result) = advance(&setup, &mut attempt);
    assert!(matches!(result, Ok(Step::Enrolled(_))), "{result:?}");
    let keys = setup.api().keys.lock().unwrap().clone();
    assert_eq!(keys.len(), 4);
    assert!(keys.iter().all(|key| *key == keys[0]));
    assert_eq!(stored_secret(&setup), platform_secret(2));
    assert!(!setup.api().live().contains(&platform_secret(1)));
}

#[test]
fn a_machine_that_cannot_take_the_credential_keeps_it_and_the_retry_asks_oxy_nothing() {
    let (setup, _dir) = setup(FakeApi::default(), 2);
    // The first account is made the administrator: that fails after the redemption.
    setup.users().fail(UserStep::MakeAdministrator);
    let mut attempt = setup.start(request()).unwrap();
    let (_, result) = advance(&setup, &mut attempt);
    assert!(matches!(result, Err(SetupError::Failed(_))), "{result:?}");
    // Undone on the machine, the platform credential kept, not ended online.
    assert!(setup.users().names().is_empty());
    assert_eq!(files(&setup), 0);
    assert!(attempt.holds_credential());
    assert!(!setup.api().calls().contains(&Call::SignOut));
    assert!(setup.api().is_pending(&platform_secret(1)));
    let asked = setup.api().calls().len();
    setup.users().failing.lock().unwrap().clear();
    let (stages, result) = advance(&setup, &mut attempt);
    assert!(matches!(result, Ok(Step::Enrolled(_))), "{result:?}");
    // Nothing asked of Oxy but the confirmation: the kept credential set up the machine.
    assert_eq!(setup.api().calls()[asked..], [Call::Confirm]);
    assert_eq!(
        stages,
        [Stage::Preparing, Stage::Finishing, Stage::Confirming]
    );
    assert_eq!(stored_secret(&setup), platform_secret(1));
    assert!(!attempt.holds_credential());
    assert!(setup.users().is_admin(account().user_name().as_str()));
}

#[test]
fn a_setup_abandoned_with_a_kept_credential_ends_nothing_online() {
    let (setup, _dir) = setup(FakeApi::default(), 2);
    setup.users().fail(UserStep::MakeAdministrator);
    let mut attempt = setup.start(request()).unwrap();
    let (_, result) = advance(&setup, &mut attempt);
    assert!(matches!(result, Err(SetupError::Failed(_))), "{result:?}");
    setup.abandon(&mut attempt);
    assert!(!attempt.holds_credential());
    // The credential it was issued is pending at Oxy and expires by itself (R8).
    let calls = setup.api().calls();
    assert!(!calls.contains(&Call::SignOut));
    assert!(!calls.contains(&Call::Confirm));
    assert!(setup.api().is_pending(&platform_secret(1)));
    assert!(setup.users().names().is_empty());
    assert_eq!(files(&setup), 0);
}

#[test]
fn a_redemption_refused_on_a_retry_is_sign_in_again() {
    // No automatic asks again: the answer lost is the person's retry's to repeat.
    let (setup, _dir) = setup(FakeApi::default(), 0);
    setup.api().hiccup(&[Hiccup::AnswerLost]);
    let mut attempt = setup.start(request()).unwrap();
    let (_, result) = advance(&setup, &mut attempt);
    assert_eq!(result.unwrap_err(), SetupError::Offline);
    // Oxy no longer answers that redemption (its ten minutes passed): `invalid_grant`.
    setup.api().redemptions.lock().unwrap().clear();
    let (_, result) = advance(&setup, &mut attempt);
    assert_eq!(result.unwrap_err(), SetupError::CredentialRefused);
    assert!(setup.users().names().is_empty());
    assert_eq!(files(&setup), 0);
}

#[test]
fn another_key_redeems_nothing() {
    let (setup, _dir) = setup(FakeApi::default(), 0);
    setup.api().hiccup(&[Hiccup::AnswerLost]);
    let mut attempt = setup.start(request()).unwrap();
    let (_, result) = advance(&setup, &mut attempt);
    assert_eq!(result.unwrap_err(), SetupError::Offline);
    // Whoever saw the proof (the setup client did) asks with a key of its own.
    let mut thief = setup.start(request()).unwrap();
    let (_, result) = advance(&setup, &mut thief);
    assert_eq!(result.unwrap_err(), SetupError::CredentialRefused);
    // The setup's own key still redeems it.
    let (_, result) = advance(&setup, &mut attempt);
    assert!(matches!(result, Ok(Step::Enrolled(_))), "{result:?}");
}

#[test]
fn a_new_home_starts_with_the_appearance_chosen() {
    let (setup, _dir) = setup(FakeApi::default(), 0);
    let mut chosen = request();
    chosen.appearance = Some(Appearance::Dark);
    let mut attempt = setup.start(chosen).unwrap();
    let (_, result) = advance(&setup, &mut attempt);
    assert!(matches!(result, Ok(Step::Enrolled(_))), "{result:?}");
    let name = account().user_name();
    let file = std::path::PathBuf::from(format!("/home/{name}/.config/oxyos/preferences"));
    let seeded = setup.users().seeded.lock().unwrap().clone();
    let text = seeded.get(&file).expect("the preferences seeded");
    assert_eq!(
        oxyos_preferences::Preferences::parse(text).appearance,
        oxyos_preferences::Appearance::Dark
    );
}

#[test]
fn an_account_set_up_again_keeps_its_own_appearance() {
    let (setup, _dir) = setup(FakeApi::default(), 0);
    let mut first = request();
    first.appearance = Some(Appearance::Light);
    setup.enroll(first).unwrap();
    setup.users().seeded.lock().unwrap().clear();
    let mut again = request();
    again.credential.device_secret = Secret::new(OTHER_DEVICE_SECRET);
    again.appearance = Some(Appearance::Dark);
    setup.enroll(again).unwrap();
    assert!(setup.users().seeded.lock().unwrap().is_empty());
    // Only the first setup, which made the home, seeded it.
    assert_eq!(
        setup
            .users()
            .steps()
            .iter()
            .filter(|step| **step == UserStep::SeedHome)
            .count(),
        1
    );
}

#[test]
fn a_home_that_cannot_be_seeded_undoes_the_setup_and_keeps_the_sign_in() {
    let (setup, _dir) = setup(FakeApi::default(), 0);
    setup.users().fail(UserStep::SeedHome);
    let mut chosen = request();
    chosen.appearance = Some(Appearance::Auto);
    let mut attempt = setup.start(chosen).unwrap();
    let (_, result) = advance(&setup, &mut attempt);
    assert!(matches!(result, Err(SetupError::Failed(_))), "{result:?}");
    assert!(setup.users().names().is_empty());
    assert_eq!(files(&setup), 0);
    assert!(attempt.holds_credential());
}

/// The steps and the waits a turn told, then its result.
fn told(
    setup: &Setup<FakeApi, FakeUsers>,
    attempt: &mut Attempt,
) -> (Vec<Event>, Result<Step, SetupError>) {
    let mut events = Vec::new();
    let result = setup.advance(attempt, &mut |event| events.push(event));
    (events, result)
}

#[test]
fn a_confirmation_oxy_cannot_be_reached_for_keeps_the_setup_and_a_retry_only_confirms() {
    let (setup, _dir) = setup(FakeApi::default(), 1);
    setup
        .api()
        .confirm_hiccups
        .lock()
        .unwrap()
        .extend([Hiccup::Unreachable, Hiccup::Unavailable]);
    let mut attempt = setup.start(request()).unwrap();
    let (_, result) = advance(&setup, &mut attempt);
    assert_eq!(result.unwrap_err(), SetupError::Offline);
    // The machine took the account; Oxy has not confirmed it.
    assert!(attempt.awaits_confirmation());
    assert_eq!(setup.users().names().len(), 1);
    assert_eq!(stored_secret(&setup), platform_secret(1));
    assert!(setup.api().is_pending(&platform_secret(1)));
    let asked = setup.api().calls().len();
    let (stages, result) = advance(&setup, &mut attempt);
    assert!(matches!(result, Ok(Step::Enrolled(_))), "{result:?}");
    assert_eq!(stages, [Stage::Confirming]);
    assert_eq!(setup.api().calls()[asked..], [Call::Confirm]);
    assert!(setup.api().live().contains(&platform_secret(1)));
    assert!(!attempt.awaits_confirmation());
}

#[test]
fn a_confirmation_whose_answer_is_lost_is_asked_again_and_holds() {
    let (setup, _dir) = setup(FakeApi::default(), 2);
    setup
        .api()
        .confirm_hiccups
        .lock()
        .unwrap()
        .push_back(Hiccup::AnswerLost);
    let mut attempt = setup.start(request()).unwrap();
    let (_, result) = advance(&setup, &mut attempt);
    assert!(matches!(result, Ok(Step::Enrolled(_))), "{result:?}");
    // Confirmed twice, the same answer: it is the machine's.
    let confirms = setup
        .api()
        .calls()
        .iter()
        .filter(|call| **call == Call::Confirm)
        .count();
    assert_eq!(confirms, 2);
    assert!(setup.api().live().contains(&platform_secret(1)));
}

#[test]
fn a_credential_oxy_will_not_confirm_undoes_the_setup_and_is_sign_in_again() {
    let (setup, _dir) = setup(FakeApi::default(), 0);
    setup
        .api()
        .confirm_hiccups
        .lock()
        .unwrap()
        .push_back(Hiccup::Unreachable);
    let mut attempt = setup.start(request()).unwrap();
    let (_, result) = advance(&setup, &mut attempt);
    assert_eq!(result.unwrap_err(), SetupError::Offline);
    // Its ten minutes pass before the retry.
    setup.api().expire();
    let (_, result) = advance(&setup, &mut attempt);
    assert_eq!(result.unwrap_err(), SetupError::CredentialRefused);
    assert!(!attempt.awaits_confirmation());
    assert!(setup.users().names().is_empty());
    assert_eq!(files(&setup), 0);
    // The first account was made the administrator, never recorded: the next one is.
    assert!(!setup.store().fact(crate::store::ADMINISTRATOR_MADE));
}

#[test]
fn an_account_set_up_again_that_oxy_will_not_confirm_is_put_back_as_it_was() {
    let (setup, _dir) = setup(FakeApi::default(), 0);
    setup.enroll(request()).unwrap();
    let before = setup.store().get(&account()).unwrap().unwrap();
    let mut again = request();
    again.credential.device_secret = Secret::new(OTHER_DEVICE_SECRET);
    setup
        .api()
        .confirm_hiccups
        .lock()
        .unwrap()
        .push_back(Hiccup::Unreachable);
    let mut attempt = setup.start(again).unwrap();
    let (_, result) = advance(&setup, &mut attempt);
    assert_eq!(result.unwrap_err(), SetupError::Offline);
    setup.api().expire();
    let (_, result) = advance(&setup, &mut attempt);
    assert_eq!(result.unwrap_err(), SetupError::CredentialRefused);
    assert_eq!(setup.store().get(&account()).unwrap(), Some(before));
    // The credential it had was not ended: it is still the account's.
    assert!(!setup.api().calls().contains(&Call::SignOut));
}

#[test]
fn a_setup_abandoned_before_its_confirmation_is_kept() {
    let (setup, _dir) = setup(FakeApi::default(), 0);
    setup
        .api()
        .confirm_hiccups
        .lock()
        .unwrap()
        .push_back(Hiccup::AnswerLost);
    let mut attempt = setup.start(request()).unwrap();
    let (_, result) = advance(&setup, &mut attempt);
    assert_eq!(result.unwrap_err(), SetupError::Offline);
    setup.abandon(&mut attempt);
    // Oxy confirmed it (its answer lost): the account works, nothing ended online.
    assert_eq!(stored_secret(&setup), platform_secret(1));
    assert!(setup.api().live().contains(&platform_secret(1)));
    assert!(!setup.api().calls().contains(&Call::SignOut));
    assert!(setup.store().fact(crate::store::ADMINISTRATOR_MADE));
}

#[test]
fn a_redemption_oxy_asks_to_wait_is_waited_told_and_asked_again_with_its_key() {
    let (setup, _dir) = setup(FakeApi::default(), 0);
    setup.api().limited.lock().unwrap().extend([Some(30), None]);
    let mut attempt = setup.start(request()).unwrap();
    let (events, result) = told(&setup, &mut attempt);
    assert!(matches!(result, Ok(Step::Enrolled(_))), "{result:?}");
    let waits: Vec<_> = events
        .iter()
        .filter(|event| matches!(event, Event::Waiting(_)))
        .collect();
    // Oxy's wait, then the default one where it said none.
    assert_eq!(waits, [&Event::Waiting(30), &Event::Waiting(30)]);
    let at = |wanted: Event| events.iter().position(|event| *event == wanted).unwrap();
    assert!(at(Event::Started(Stage::SigningIn)) < at(Event::Waiting(30)));
    assert!(at(Event::Waiting(30)) < at(Event::Started(Stage::Finishing)));
    let keys = setup.api().keys.lock().unwrap().clone();
    assert_eq!(keys.len(), 3);
    assert!(keys.iter().all(|key| *key == keys[0]));
}

#[test]
fn a_wait_that_would_outlast_the_window_is_sign_in_again() {
    let (setup, _dir) = setup(FakeApi::default(), 0);
    setup.api().limited.lock().unwrap().push_back(Some(900));
    let mut attempt = setup.start(request()).unwrap();
    let (events, result) = told(&setup, &mut attempt);
    assert_eq!(result.unwrap_err(), SetupError::CredentialRefused);
    assert!(!events
        .iter()
        .any(|event| matches!(event, Event::Waiting(_))));
    assert!(setup.users().names().is_empty());
    assert_eq!(files(&setup), 0);
}
