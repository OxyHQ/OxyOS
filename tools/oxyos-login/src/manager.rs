//! What the login program decides, apart from doing it (`run.rs` does): when the login screen
//! runs, which password is checked, what the screen is told, and in what order the seat passes
//! from the login screen to the person's session and back. It is a state machine fed what
//! happened, answering with what to do, so every order is tested without a seat, PAM or a
//! process.
//!
//! - **One login screen, one check at a time.** A password is checked only while the login screen
//!   runs and nothing else is checked; another is `Busy`.
//! - **Only an enrolled account.** An account the accounts' service does not list is refused
//!   without PAM (`enrolled: false`), throttled together with every other such name.
//! - **The online check** (ADR 0020 decision 2, step 3). Once the worker's PAM has answered, the
//!   accounts' service is asked with that answer (`check`), and its answer stands: a password PAM
//!   rejected that Oxy accepts (changed elsewhere; the service has replaced the verifier) is
//!   checked by PAM again with a new worker, which then opens the session; a password PAM
//!   accepted is refused only when the service says so (an account Oxy signed out, or a password
//!   Oxy says was changed while it can ask). The service unreachable, nobody signs in
//!   (decision 7), and that is no guess.
//! - **The throttle** is the lock's (`oxyos_auth::Throttle`, ADR 0010), one per account: five
//!   rejections free, then 30 s doubling to 15 min; an acceptance resets it. A failure that was no
//!   guess (PAM broken, the worker gone, the service unreachable) rejects without counting.
//! - **The handover.** Accepted: the login screen is told, shows the splash and ends; if it has
//!   not within [`GREETER_GRACE`] it is stopped, and killed after as long again. Only once it has
//!   ended (its logind session closed, the card's DRM master let go) is the person's session
//!   opened. When that session ends (signing out), or fails to open, the login screen starts
//!   again.
//! - **A login screen that ends** while nobody signs in (it crashed) is started again, no sooner
//!   than [`RESTART`] after it last started; a check it had going is abandoned.

use oxyos_auth::greeter::Verdict;
use oxyos_auth::{Outcome, Throttle};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// How long the login screen has to end once someone signed in, before it is stopped (and as
/// long again before it is killed).
pub const GREETER_GRACE: Duration = Duration::from_secs(5);
/// A login screen that ended is started again no sooner than this after it last started.
pub const RESTART: Duration = Duration::from_secs(1);

/// What a worker said of a password.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Checked {
    Accepted,
    Rejected,
    /// PAM failed, or the worker went: no guess was made.
    Failed,
}

/// What PAM said of a password, as the accounts' service is told it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Local {
    Accepted,
    Rejected,
}

/// What the accounts' service answered of a password.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Online {
    Accepted,
    Rejected,
    /// The service did not answer: nobody signs in, and it was no guess.
    Unreachable,
}

/// Where a password's check is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// A worker's PAM checks it; `online_accepted`: the service already accepted it (the
    /// verifier was replaced), so PAM's answer is the last.
    Pam { online_accepted: bool },
    /// The accounts' service checks it after PAM said `local` (the worker, if PAM accepted,
    /// waits to open the session).
    Online { local: Local },
}

/// The password being checked: for which account, how far, and the online check it waits for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checking {
    pub account: String,
    pub step: Step,
    pub check: u64,
}

/// What the login program does next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Starts the login screen (its worker opens its greeter session), after `after`.
    StartGreeter { after: Duration },
    /// Asks the login screen to end (its worker stops it and closes its session).
    StopGreeter,
    /// Kills the login screen's worker, and so the login screen.
    KillGreeter,
    /// Tells the login screen what came of its attempt.
    Answer { verdict: Verdict, wait_s: u32 },
    /// Starts a worker for `account` and gives it the password the login screen sent.
    Check { account: String },
    /// Asks the accounts' service to check the password online, PAM having said `local`; its
    /// answer comes back with `check` ([`Manager::online_checked`]).
    CheckOnline {
        account: String,
        local: Local,
        check: u64,
    },
    /// Ends the worker checking a password, unanswered.
    AbandonCheck,
    /// Tells the worker whose password was accepted to open the session and start it.
    OpenSession,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// The login screen is starting or running; `checking` the password being checked.
    Greeter { checking: Option<Checking> },
    /// `account` signed in; the login screen is ending. `stopped`: it was asked to.
    HandingOver {
        account: String,
        until: Instant,
        stopped: bool,
    },
    /// The login screen has ended; the worker is opening `account`'s session.
    Opening { account: String },
    /// `account`'s session runs.
    Session { account: String },
}

pub struct Manager {
    state: State,
    /// Each enrolled account's throttle, and one for every name not enrolled.
    throttles: BTreeMap<String, Throttle>,
    unknown: Throttle,
    greeter_started: Option<Instant>,
    /// The last online check asked: an answer to an earlier one is no answer.
    checks: u64,
}

/// Whole seconds, rounded up: a wait of 0.2 s is still a wait.
fn seconds(wait: Duration) -> u32 {
    u32::try_from(wait.as_millis().div_ceil(1000)).unwrap_or(u32::MAX)
}

impl Manager {
    /// The login program starting: the login screen first.
    pub fn start(now: Instant) -> (Self, Vec<Action>) {
        let manager = Self {
            state: State::Greeter { checking: None },
            throttles: BTreeMap::new(),
            unknown: Throttle::default(),
            greeter_started: Some(now),
            checks: 0,
        };
        (
            manager,
            vec![Action::StartGreeter {
                after: Duration::ZERO,
            }],
        )
    }

    #[cfg(test)]
    pub fn state(&self) -> &State {
        &self.state
    }

    fn throttle(&mut self, account: &str, enrolled: bool) -> &mut Throttle {
        if enrolled {
            self.throttles.entry(account.to_owned()).or_default()
        } else {
            &mut self.unknown
        }
    }

    /// The login screen sent a password for `account`; `enrolled`: the accounts' service lists it.
    pub fn attempt(&mut self, account: &str, enrolled: bool, now: Instant) -> Vec<Action> {
        let busy = vec![Action::Answer {
            verdict: Verdict::Busy,
            wait_s: 0,
        }];
        let State::Greeter { checking } = &self.state else {
            return busy;
        };
        if checking.is_some() {
            return busy;
        }
        let throttle = self.throttle(account, enrolled);
        if let Some(wait) = throttle.wait(now) {
            return vec![Action::Answer {
                verdict: Verdict::Throttled,
                wait_s: seconds(wait),
            }];
        }
        if !enrolled {
            throttle.record(Outcome::Rejected, now);
            let wait_s = seconds(throttle.wait(now).unwrap_or_default());
            return vec![Action::Answer {
                verdict: Verdict::Rejected,
                wait_s,
            }];
        }
        self.state = State::Greeter {
            checking: Some(Checking {
                account: account.to_owned(),
                step: Step::Pam {
                    online_accepted: false,
                },
                check: self.checks,
            }),
        };
        vec![Action::Check {
            account: account.to_owned(),
        }]
    }

    /// The password is accepted: the login screen is told, and ends.
    fn accept(&mut self, account: String, now: Instant) -> Vec<Action> {
        self.throttle(&account, true).record(Outcome::Accepted, now);
        self.state = State::HandingOver {
            account,
            until: now + GREETER_GRACE,
            stopped: false,
        };
        vec![Action::Answer {
            verdict: Verdict::Accepted,
            wait_s: 0,
        }]
    }

    /// The password is refused; `counted`: it was a guess, which the throttle counts.
    fn reject(&mut self, account: &str, counted: bool, now: Instant) -> Vec<Action> {
        let throttle = self.throttle(account, true);
        if counted {
            throttle.record(Outcome::Rejected, now);
        }
        let wait_s = seconds(throttle.wait(now).unwrap_or_default());
        self.state = State::Greeter { checking: None };
        vec![Action::Answer {
            verdict: Verdict::Rejected,
            wait_s,
        }]
    }

    /// The online check of PAM's `local` answer, asked.
    fn ask_online(&mut self, account: String, local: Local) -> Vec<Action> {
        self.checks += 1;
        let check = self.checks;
        self.state = State::Greeter {
            checking: Some(Checking {
                account: account.clone(),
                step: Step::Online { local },
                check,
            }),
        };
        vec![Action::CheckOnline {
            account,
            local,
            check,
        }]
    }

    /// The worker checking a password answered.
    pub fn checked(&mut self, checked: Checked, now: Instant) -> Vec<Action> {
        let State::Greeter {
            checking:
                Some(Checking {
                    account,
                    step: Step::Pam { online_accepted },
                    ..
                }),
        } = &self.state
        else {
            return Vec::new();
        };
        let (account, online_accepted) = (account.clone(), *online_accepted);
        match (checked, online_accepted) {
            // The service has accepted it already: PAM's answer is the last.
            (Checked::Accepted, true) => self.accept(account, now),
            (Checked::Rejected, true) => self.reject(&account, true, now),
            (Checked::Accepted, false) => self.ask_online(account, Local::Accepted),
            (Checked::Rejected, false) => self.ask_online(account, Local::Rejected),
            (Checked::Failed, _) => self.reject(&account, false, now),
        }
    }

    /// The accounts' service answered online check `check`.
    pub fn online_checked(&mut self, check: u64, online: Online, now: Instant) -> Vec<Action> {
        let State::Greeter {
            checking:
                Some(Checking {
                    account,
                    step: Step::Online { local },
                    check: asked,
                }),
        } = &self.state
        else {
            return Vec::new();
        };
        if *asked != check {
            return Vec::new();
        }
        let (account, local) = (account.clone(), *local);
        // A worker waiting to open the session it accepted is ended unless it opens one.
        let abandon = |mut actions: Vec<Action>| {
            if local == Local::Accepted {
                actions.insert(0, Action::AbandonCheck);
            }
            actions
        };
        match (online, local) {
            (Online::Accepted, Local::Accepted) => self.accept(account, now),
            // Changed elsewhere, and the verifier replaced: PAM checks it again, and its worker
            // opens the session.
            (Online::Accepted, Local::Rejected) => {
                self.state = State::Greeter {
                    checking: Some(Checking {
                        account: account.clone(),
                        step: Step::Pam {
                            online_accepted: true,
                        },
                        check,
                    }),
                };
                vec![Action::Check { account }]
            }
            (Online::Rejected, _) => abandon(self.reject(&account, true, now)),
            (Online::Unreachable, _) => abandon(self.reject(&account, false, now)),
        }
    }

    /// The login screen's worker ended: the login screen has gone and its session is closed.
    pub fn greeter_ended(&mut self, now: Instant) -> Vec<Action> {
        match &self.state {
            State::HandingOver { account, .. } => {
                self.state = State::Opening {
                    account: account.clone(),
                };
                vec![Action::OpenSession]
            }
            State::Greeter { checking } => {
                // It crashed: started again, a check it had going abandoned.
                let mut actions = Vec::new();
                if checking.is_some() {
                    actions.push(Action::AbandonCheck);
                }
                self.state = State::Greeter { checking: None };
                actions.push(self.restart(now));
                actions
            }
            State::Opening { .. } | State::Session { .. } => Vec::new(),
        }
    }

    fn restart(&mut self, now: Instant) -> Action {
        let after = self
            .greeter_started
            .map(|started| (started + RESTART).saturating_duration_since(now))
            .unwrap_or_default();
        self.greeter_started = Some(now + after);
        Action::StartGreeter { after }
    }

    /// The worker that opened the session started it.
    pub fn session_started(&mut self) -> Vec<Action> {
        if let State::Opening { account } = &self.state {
            self.state = State::Session {
                account: account.clone(),
            };
        }
        Vec::new()
    }

    /// The worker of a password or a session ended.
    pub fn worker_ended(&mut self, now: Instant) -> Vec<Action> {
        match &self.state {
            // Checking, it went without an answer: no guess, and nobody signed in.
            State::Greeter {
                checking:
                    Some(Checking {
                        step: Step::Pam { .. },
                        ..
                    }),
            } => self.checked(Checked::Failed, now),
            // It rejected the password and ended, as a worker does: the service is being asked.
            State::Greeter {
                checking:
                    Some(Checking {
                        step:
                            Step::Online {
                                local: Local::Rejected,
                            },
                        ..
                    }),
            } => Vec::new(),
            // It accepted, and went before the service answered: there is nothing to open the
            // session, so nobody signs in, and it was no guess.
            State::Greeter {
                checking:
                    Some(Checking {
                        account,
                        step:
                            Step::Online {
                                local: Local::Accepted,
                            },
                        ..
                    }),
            } => {
                let account = account.clone();
                self.reject(&account, false, now)
            }
            // The session ended (signed out), or never opened: the login screen again.
            State::Opening { .. } | State::Session { .. } => {
                self.state = State::Greeter { checking: None };
                vec![self.restart(now)]
            }
            // The accepted worker went before its session opened: the login screen is ending,
            // and starts again once it has.
            State::HandingOver { .. } => {
                self.state = State::Greeter { checking: None };
                Vec::new()
            }
            State::Greeter { checking: None } => Vec::new(),
        }
    }

    /// Time passed: a login screen slow to end is stopped, then killed.
    pub fn tick(&mut self, now: Instant) -> Vec<Action> {
        let State::HandingOver { until, stopped, .. } = &mut self.state else {
            return Vec::new();
        };
        if now < *until {
            return Vec::new();
        }
        *until = now + GREETER_GRACE;
        if std::mem::replace(stopped, true) {
            vec![Action::KillGreeter]
        } else {
            vec![Action::StopGreeter]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADA: &str = "oxy-aaaaaaaaaaaaaaaaaaaa";

    fn answer(verdict: Verdict, wait_s: u32) -> Vec<Action> {
        vec![Action::Answer { verdict, wait_s }]
    }

    /// PAM's answer, then the service's agreeing with it (online, nothing changed elsewhere).
    fn pam(manager: &mut Manager, checked: Checked, now: Instant) -> Vec<Action> {
        let actions = manager.checked(checked, now);
        match actions.as_slice() {
            [Action::CheckOnline { local, check, .. }] => {
                let online = match local {
                    Local::Accepted => Online::Accepted,
                    Local::Rejected => Online::Rejected,
                };
                manager.online_checked(*check, online, now)
            }
            _ => actions,
        }
    }

    /// PAM's answer: the online check it asks for.
    fn asks_online(manager: &mut Manager, checked: Checked, local: Local, now: Instant) -> u64 {
        let actions = manager.checked(checked, now);
        let [Action::CheckOnline {
            account,
            local: asked,
            check,
        }] = actions.as_slice()
        else {
            panic!("an online check: {actions:?}");
        };
        assert_eq!((account.as_str(), *asked), (ADA, local));
        *check
    }

    fn signed_in(manager: &mut Manager, now: Instant) {
        assert_eq!(
            manager.attempt(ADA, true, now),
            [Action::Check {
                account: ADA.into()
            }]
        );
        assert_eq!(
            pam(manager, Checked::Accepted, now),
            answer(Verdict::Accepted, 0)
        );
    }

    #[test]
    fn the_login_screen_comes_first() {
        let (manager, actions) = Manager::start(Instant::now());
        assert_eq!(
            actions,
            [Action::StartGreeter {
                after: Duration::ZERO
            }]
        );
        assert_eq!(manager.state(), &State::Greeter { checking: None });
    }

    #[test]
    fn accepted_then_the_session_then_signing_out_returns_to_the_login_screen() {
        let now = Instant::now();
        let (mut manager, _) = Manager::start(now);
        signed_in(&mut manager, now);
        // The session waits for the login screen to end: nothing opens it before.
        assert!(matches!(manager.state(), State::HandingOver { .. }));
        assert!(manager.session_started().is_empty());
        assert!(matches!(manager.state(), State::HandingOver { .. }));
        let later = now + Duration::from_secs(2);
        assert_eq!(manager.greeter_ended(later), [Action::OpenSession]);
        assert_eq!(
            manager.state(),
            &State::Opening {
                account: ADA.into()
            }
        );
        assert!(manager.session_started().is_empty());
        assert_eq!(
            manager.state(),
            &State::Session {
                account: ADA.into()
            }
        );
        // While the session runs, nothing is checked.
        assert_eq!(manager.attempt(ADA, true, later), answer(Verdict::Busy, 0));
        // Signing out: the login screen again, at once (it started long enough ago).
        let out = now + Duration::from_secs(3600);
        assert_eq!(
            manager.worker_ended(out),
            [Action::StartGreeter {
                after: Duration::ZERO
            }]
        );
        assert_eq!(manager.state(), &State::Greeter { checking: None });
        // And the next person signs in as the first did.
        signed_in(&mut manager, out);
    }

    #[test]
    fn a_rejected_password_answers_and_checks_the_next() {
        let now = Instant::now();
        let (mut manager, _) = Manager::start(now);
        manager.attempt(ADA, true, now);
        // One at a time.
        assert_eq!(manager.attempt(ADA, true, now), answer(Verdict::Busy, 0));
        assert_eq!(
            pam(&mut manager, Checked::Rejected, now),
            answer(Verdict::Rejected, 0)
        );
        assert_eq!(manager.state(), &State::Greeter { checking: None });
        assert_eq!(
            manager.attempt(ADA, true, now),
            [Action::Check {
                account: ADA.into()
            }]
        );
    }

    #[test]
    fn five_rejections_then_the_waits_and_an_acceptance_resets_them() {
        let mut now = Instant::now();
        let (mut manager, _) = Manager::start(now);
        for _ in 0..4 {
            manager.attempt(ADA, true, now);
            assert_eq!(
                pam(&mut manager, Checked::Rejected, now),
                answer(Verdict::Rejected, 0)
            );
        }
        manager.attempt(ADA, true, now);
        assert_eq!(
            pam(&mut manager, Checked::Rejected, now),
            answer(Verdict::Rejected, 30)
        );
        // Throttled: no worker is started, the wait is said.
        now += Duration::from_secs(10);
        assert_eq!(
            manager.attempt(ADA, true, now),
            answer(Verdict::Throttled, 20)
        );
        // Another account is not held back by this one's.
        let other = "oxy-bbbbbbbbbbbbbbbbbbbb";
        assert_eq!(
            manager.attempt(other, true, now),
            [Action::Check {
                account: other.into()
            }]
        );
        pam(&mut manager, Checked::Rejected, now);
        // The wait over, a sixth rejection doubles it.
        now += Duration::from_secs(20);
        manager.attempt(ADA, true, now);
        assert_eq!(
            pam(&mut manager, Checked::Rejected, now),
            answer(Verdict::Rejected, 60)
        );
        now += Duration::from_secs(60);
        signed_in(&mut manager, now);
        manager.greeter_ended(now);
        manager.session_started();
        manager.worker_ended(now);
        // Reset: five free rejections again.
        for _ in 0..4 {
            manager.attempt(ADA, true, now);
            assert_eq!(
                pam(&mut manager, Checked::Rejected, now),
                answer(Verdict::Rejected, 0)
            );
        }
    }

    #[test]
    fn a_failure_rejects_without_counting() {
        let now = Instant::now();
        let (mut manager, _) = Manager::start(now);
        for _ in 0..10 {
            manager.attempt(ADA, true, now);
            assert_eq!(
                pam(&mut manager, Checked::Failed, now),
                answer(Verdict::Rejected, 0)
            );
        }
        // A worker gone without an answer is such a failure.
        manager.attempt(ADA, true, now);
        assert_eq!(manager.worker_ended(now), answer(Verdict::Rejected, 0));
        assert_eq!(manager.state(), &State::Greeter { checking: None });
    }

    #[test]
    fn an_account_not_enrolled_is_refused_without_pam_and_throttled_together() {
        let now = Instant::now();
        let (mut manager, _) = Manager::start(now);
        for name in ["root", "nate", "oxy-x", "daemon"] {
            assert_eq!(
                manager.attempt(name, false, now),
                answer(Verdict::Rejected, 0)
            );
        }
        assert_eq!(
            manager.attempt("bin", false, now),
            answer(Verdict::Rejected, 30)
        );
        assert_eq!(
            manager.attempt("sys", false, now),
            answer(Verdict::Throttled, 30)
        );
        // An enrolled account is still checked.
        assert_eq!(
            manager.attempt(ADA, true, now),
            [Action::Check {
                account: ADA.into()
            }]
        );
    }

    #[test]
    fn a_login_screen_slow_to_end_is_stopped_then_killed_before_the_session_opens() {
        let now = Instant::now();
        let (mut manager, _) = Manager::start(now);
        signed_in(&mut manager, now);
        assert!(manager.tick(now + Duration::from_secs(1)).is_empty());
        assert_eq!(manager.tick(now + GREETER_GRACE), [Action::StopGreeter]);
        assert!(manager.tick(now + GREETER_GRACE).is_empty());
        assert_eq!(manager.tick(now + GREETER_GRACE * 2), [Action::KillGreeter]);
        // Only its end opens the session.
        assert!(matches!(manager.state(), State::HandingOver { .. }));
        assert_eq!(
            manager.greeter_ended(now + GREETER_GRACE * 2),
            [Action::OpenSession]
        );
        assert!(manager.tick(now + GREETER_GRACE * 10).is_empty());
    }

    #[test]
    fn a_login_screen_that_crashes_is_started_again_and_its_check_abandoned() {
        let now = Instant::now();
        let (mut manager, _) = Manager::start(now);
        manager.attempt(ADA, true, now);
        // Within a second of its start: started again once the second is over.
        let soon = now + Duration::from_millis(300);
        assert_eq!(
            manager.greeter_ended(soon),
            [
                Action::AbandonCheck,
                Action::StartGreeter {
                    after: Duration::from_millis(700)
                }
            ]
        );
        assert_eq!(manager.state(), &State::Greeter { checking: None });
        // The abandoned check's answer, if it comes, signs nobody in.
        assert!(pam(&mut manager, Checked::Accepted, soon).is_empty());
        assert_eq!(manager.state(), &State::Greeter { checking: None });
        // Later, at once.
        let later = now + Duration::from_secs(60);
        assert_eq!(
            manager.greeter_ended(later),
            [Action::StartGreeter {
                after: Duration::ZERO
            }]
        );
    }

    #[test]
    fn a_session_that_fails_to_open_returns_to_the_login_screen() {
        let now = Instant::now();
        let (mut manager, _) = Manager::start(now);
        signed_in(&mut manager, now);
        manager.greeter_ended(now);
        let later = now + Duration::from_secs(5);
        assert_eq!(
            manager.worker_ended(later),
            [Action::StartGreeter {
                after: Duration::ZERO
            }]
        );
        assert_eq!(manager.state(), &State::Greeter { checking: None });
    }

    #[test]
    fn an_accepted_worker_that_goes_early_opens_nothing() {
        let now = Instant::now();
        let (mut manager, _) = Manager::start(now);
        signed_in(&mut manager, now);
        assert!(manager.worker_ended(now).is_empty());
        // The login screen ending now opens no session; it is started again.
        assert_eq!(
            manager.greeter_ended(now + Duration::from_secs(2)),
            [Action::StartGreeter {
                after: Duration::ZERO
            }]
        );
    }

    #[test]
    fn pams_answer_goes_to_the_service_before_anyone_signs_in() {
        let now = Instant::now();
        let (mut manager, _) = Manager::start(now);
        manager.attempt(ADA, true, now);
        let check = asks_online(&mut manager, Checked::Accepted, Local::Accepted, now);
        // Not yet: the login screen is told nothing until the service answers.
        assert!(matches!(
            manager.state(),
            State::Greeter { checking: Some(_) }
        ));
        assert_eq!(manager.attempt(ADA, true, now), answer(Verdict::Busy, 0));
        // An answer to another check is no answer.
        assert!(manager
            .online_checked(check + 1, Online::Accepted, now)
            .is_empty());
        assert_eq!(
            manager.online_checked(check, Online::Accepted, now),
            answer(Verdict::Accepted, 0)
        );
        assert!(matches!(manager.state(), State::HandingOver { .. }));
        assert_eq!(manager.greeter_ended(now), [Action::OpenSession]);
    }

    #[test]
    fn a_password_changed_elsewhere_is_checked_by_pam_again_and_signs_in() {
        let now = Instant::now();
        let (mut manager, _) = Manager::start(now);
        manager.attempt(ADA, true, now);
        let check = asks_online(&mut manager, Checked::Rejected, Local::Rejected, now);
        // The worker that rejected it ends, as rejecting workers do: that is expected.
        assert!(manager.worker_ended(now).is_empty());
        // Oxy accepts it, and the service replaced the verifier: a new worker checks it.
        assert_eq!(
            manager.online_checked(check, Online::Accepted, now),
            [Action::Check {
                account: ADA.into()
            }]
        );
        // PAM now accepts, and its answer is the last: no second online check.
        assert_eq!(
            manager.checked(Checked::Accepted, now),
            answer(Verdict::Accepted, 0)
        );
        assert_eq!(manager.greeter_ended(now), [Action::OpenSession]);
    }

    #[test]
    fn the_service_refusing_a_password_pam_accepted_ends_its_worker_and_counts() {
        let now = Instant::now();
        let (mut manager, _) = Manager::start(now);
        for _ in 0..5 {
            manager.attempt(ADA, true, now);
            let check = asks_online(&mut manager, Checked::Accepted, Local::Accepted, now);
            let actions = manager.online_checked(check, Online::Rejected, now);
            assert_eq!(actions[0], Action::AbandonCheck);
        }
        // Counted: the fifth waits.
        assert_eq!(
            manager.attempt(ADA, true, now),
            answer(Verdict::Throttled, 30)
        );
    }

    #[test]
    fn the_service_unreachable_signs_nobody_in_and_is_not_counted() {
        let now = Instant::now();
        let (mut manager, _) = Manager::start(now);
        for _ in 0..10 {
            manager.attempt(ADA, true, now);
            let check = asks_online(&mut manager, Checked::Accepted, Local::Accepted, now);
            assert_eq!(
                manager.online_checked(check, Online::Unreachable, now),
                [
                    Action::AbandonCheck,
                    Action::Answer {
                        verdict: Verdict::Rejected,
                        wait_s: 0
                    }
                ]
            );
            assert_eq!(manager.state(), &State::Greeter { checking: None });
        }
        // A PAM rejection whose online check cannot be asked: refused, no worker to end.
        manager.attempt(ADA, true, now);
        let check = asks_online(&mut manager, Checked::Rejected, Local::Rejected, now);
        assert_eq!(
            manager.online_checked(check, Online::Unreachable, now),
            answer(Verdict::Rejected, 0)
        );
    }

    #[test]
    fn an_accepting_worker_gone_before_the_service_answered_opens_nothing() {
        let now = Instant::now();
        let (mut manager, _) = Manager::start(now);
        manager.attempt(ADA, true, now);
        let check = asks_online(&mut manager, Checked::Accepted, Local::Accepted, now);
        assert_eq!(manager.worker_ended(now), answer(Verdict::Rejected, 0));
        // The service's late acceptance signs nobody in.
        assert!(manager
            .online_checked(check, Online::Accepted, now)
            .is_empty());
        assert_eq!(manager.state(), &State::Greeter { checking: None });
    }
}
