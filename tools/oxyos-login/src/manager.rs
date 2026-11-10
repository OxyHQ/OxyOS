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
//! - **A PIN** (ADR 0020 decision 2): the login screen says when what it sends is the account's
//!   PIN for this computer. Its worker checks it under the PIN's own PAM service
//!   (`oxyos-login-pin`, which asks the accounts' service, offline), and its answer is the last:
//!   a PIN is never checked online, nor taken for the password.
//! - **The throttle** is the lock's (`oxyos_auth::Throttle`, ADR 0010), one per account and one
//!   per account's PIN: five rejections free, then 30 s doubling to 15 min; an acceptance resets
//!   it. A failure that was no guess (PAM broken, the worker gone, the service unreachable)
//!   rejects without counting.
//! - **The handover.** Accepted: the login screen is told, shows the splash and ends; if it has
//!   not within [`GREETER_GRACE`] it is stopped, and killed after as long again. Only once it has
//!   ended (its logind session closed, the card's DRM master let go) is the person's session
//!   opened. When that session ends (signing out), or fails to open, the login screen starts
//!   again.
//! - **A login screen that ends** while nobody signs in (it crashed) is started again with the
//!   shell's doubling wait (`oxyos_seat::restarts::Restarts`, docs/adr/0061, decision 6): at
//!   once the first time, then after 1, 2, 4 … up to 60 s while it keeps ending within a minute
//!   of its start, never given up on; a check it had going is abandoned. A session that ended
//!   (signing out) brings it back at once.

use crate::channel::End;
use oxyos_auth::greeter::Verdict;
use oxyos_auth::{Outcome, Throttle};
use oxyos_seat::restarts::Restarts;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// How long the login screen has to end once someone signed in, before it is stopped (and as
/// long again before it is killed).
pub const GREETER_GRACE: Duration = Duration::from_secs(5);

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
    /// The account's PIN, not its password.
    pub passcode: bool,
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
    /// Starts a worker for `account` and gives it the password the login screen sent, under the
    /// PIN's PAM service when it is the account's PIN.
    Check { account: String, passcode: bool },
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
    /// `account` signed in (with its PIN: `passcode`); the login screen is ending. `stopped`: it
    /// was asked to.
    HandingOver {
        account: String,
        passcode: bool,
        until: Instant,
        stopped: bool,
    },
    /// The login screen has ended; the worker is opening `account`'s session.
    Opening { account: String },
    /// `account`'s session runs.
    Session { account: String },
}

/// A session that ended because of a problem (docs/adr/0065-the-session-ending-because-of-a-problem.mdx,
/// decision 2): whose, how, after how long and when (seconds since the epoch).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub account: String,
    pub end: End,
    pub ran: u32,
    pub at: u64,
}

impl Problem {
    /// As the person's next session is told it (`OXYOS_SESSION_ENDED`): how, how many seconds it
    /// ran, and when, `signal 11 3600 1762790000`.
    pub fn told(&self) -> String {
        format!("{} {} {}", self.end.word(), self.ran, self.at)
    }
}

pub struct Manager {
    state: State,
    /// The last session that ended because of a problem, until its account's next session is
    /// told; and whether the login screen says so (until someone signs in).
    problem: Option<Problem>,
    problem_said: bool,
    /// Each enrolled account's throttle, its password's and its PIN's (`true`), and one for every
    /// name not enrolled.
    throttles: BTreeMap<(String, bool), Throttle>,
    unknown: Throttle,
    greeter_started: Option<Instant>,
    /// The login screen's ends in a row without its having run a minute.
    restarts: Restarts,
    /// The last online check asked: an answer to an earlier one is no answer.
    checks: u64,
}

impl Manager {
    /// The login program starting: the login screen first.
    pub fn start(now: Instant) -> (Self, Vec<Action>) {
        let manager = Self {
            state: State::Greeter { checking: None },
            problem: None,
            problem_said: false,
            throttles: BTreeMap::new(),
            unknown: Throttle::default(),
            greeter_started: Some(now),
            restarts: Restarts::default(),
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

    /// The account whose password was accepted, while its session is being handed the seat.
    pub fn handing_over(&self) -> Option<&str> {
        match &self.state {
            State::HandingOver { account, .. } => Some(account),
            _ => None,
        }
    }

    /// Whether the account being handed the seat signed in with its PIN.
    pub fn handing_over_with_pin(&self) -> bool {
        matches!(self.state, State::HandingOver { passcode: true, .. })
    }

    fn throttle(&mut self, account: &str, passcode: bool, enrolled: bool) -> &mut Throttle {
        if enrolled {
            self.throttles
                .entry((account.to_owned(), passcode))
                .or_default()
        } else {
            &mut self.unknown
        }
    }

    /// The login screen sent a password (its PIN: `passcode`) for `account`; `enrolled`: the
    /// accounts' service lists it.
    pub fn attempt(
        &mut self,
        account: &str,
        passcode: bool,
        enrolled: bool,
        now: Instant,
    ) -> Vec<Action> {
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
        let throttle = self.throttle(account, passcode, enrolled);
        if let Some(wait) = throttle.wait(now) {
            return vec![Action::Answer {
                verdict: Verdict::Throttled,
                wait_s: oxyos_auth::whole_seconds(wait),
            }];
        }
        if !enrolled {
            throttle.record(Outcome::Rejected, now);
            let wait_s = oxyos_auth::whole_seconds(throttle.wait(now).unwrap_or_default());
            return vec![Action::Answer {
                verdict: Verdict::Rejected,
                wait_s,
            }];
        }
        self.state = State::Greeter {
            checking: Some(Checking {
                account: account.to_owned(),
                passcode,
                step: Step::Pam {
                    online_accepted: false,
                },
                check: self.checks,
            }),
        };
        vec![Action::Check {
            account: account.to_owned(),
            passcode,
        }]
    }

    /// The password is accepted: the login screen is told, and ends.
    fn accept(&mut self, account: String, passcode: bool, now: Instant) -> Vec<Action> {
        self.throttle(&account, passcode, true)
            .record(Outcome::Accepted, now);
        self.state = State::HandingOver {
            account,
            passcode,
            until: now + GREETER_GRACE,
            stopped: false,
        };
        vec![Action::Answer {
            verdict: Verdict::Accepted,
            wait_s: 0,
        }]
    }

    /// The password is refused; `counted`: it was a guess, which the throttle counts.
    fn reject(
        &mut self,
        account: &str,
        passcode: bool,
        counted: bool,
        now: Instant,
    ) -> Vec<Action> {
        let throttle = self.throttle(account, passcode, true);
        if counted {
            throttle.record(Outcome::Rejected, now);
        }
        let wait_s = oxyos_auth::whole_seconds(throttle.wait(now).unwrap_or_default());
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
                passcode: false,
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
                    passcode,
                    step: Step::Pam { online_accepted },
                    ..
                }),
        } = &self.state
        else {
            return Vec::new();
        };
        let (account, passcode, online_accepted) = (account.clone(), *passcode, *online_accepted);
        if passcode {
            // A PIN is this computer's alone: PAM's answer is the last, never asked online.
            return match checked {
                Checked::Accepted => self.accept(account, true, now),
                Checked::Rejected => self.reject(&account, true, true, now),
                Checked::Failed => self.reject(&account, true, false, now),
            };
        }
        match (checked, online_accepted) {
            // The service has accepted it already: PAM's answer is the last.
            (Checked::Accepted, true) => self.accept(account, false, now),
            (Checked::Rejected, true) => self.reject(&account, false, true, now),
            (Checked::Accepted, false) => self.ask_online(account, Local::Accepted),
            (Checked::Rejected, false) => self.ask_online(account, Local::Rejected),
            (Checked::Failed, _) => self.reject(&account, false, false, now),
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
                    ..
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
            (Online::Accepted, Local::Accepted) => self.accept(account, false, now),
            // Changed elsewhere, and the verifier replaced: PAM checks it again, and its worker
            // opens the session.
            (Online::Accepted, Local::Rejected) => {
                self.state = State::Greeter {
                    checking: Some(Checking {
                        account: account.clone(),
                        passcode: false,
                        step: Step::Pam {
                            online_accepted: true,
                        },
                        check,
                    }),
                };
                vec![Action::Check {
                    account,
                    passcode: false,
                }]
            }
            (Online::Rejected, _) => abandon(self.reject(&account, false, true, now)),
            (Online::Unreachable, _) => abandon(self.reject(&account, false, false, now)),
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
                let ran = self.greeter_started.map_or(Duration::ZERO, |started| {
                    now.saturating_duration_since(started)
                });
                let after = self.restarts.ended(ran);
                actions.push(self.start_greeter(now, after));
                actions
            }
            State::Opening { .. } | State::Session { .. } => Vec::new(),
        }
    }

    /// The login screen started again `after` from `now`.
    fn start_greeter(&mut self, now: Instant, after: Duration) -> Action {
        self.greeter_started = Some(now + after);
        Action::StartGreeter { after }
    }

    /// The session's worker said how it ended, at `at` (seconds since the epoch), after `ran`
    /// seconds: one that ended because of a problem is said on the login screen, and told to its
    /// account's next session.
    pub fn session_ended(&mut self, end: End, ran: u32, at: u64) {
        let (State::Session { account } | State::Opening { account }) = &self.state else {
            return;
        };
        if end.problem() {
            self.problem = Some(Problem {
                account: account.clone(),
                end,
                ran,
                at,
            });
            self.problem_said = true;
        }
    }

    /// Whether the login screen says the last session ended because of a problem.
    pub fn problem_said(&self) -> bool {
        self.problem_said
    }

    /// How `account`'s last session ended because of a problem, to tell the session that starts
    /// for it (its worker is started for the attempt, before anyone knows it will sign in).
    pub fn problem_of(&self, account: &str) -> Option<&Problem> {
        self.problem
            .as_ref()
            .filter(|problem| problem.account == account)
    }

    /// The worker that opened the session started it.
    pub fn session_started(&mut self) -> Vec<Action> {
        if let State::Opening { account } = &self.state {
            // Told to it (its worker was started with it), and said no more on the login screen.
            self.problem_said = false;
            self.problem.take_if(|problem| problem.account == *account);
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
                self.reject(&account, false, false, now)
            }
            // The session ended (signed out), or never opened: the login screen again.
            State::Opening { .. } | State::Session { .. } => {
                self.state = State::Greeter { checking: None };
                vec![self.start_greeter(now, Duration::ZERO)]
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
            manager.attempt(ADA, false, true, now),
            [Action::Check {
                account: ADA.into(),
                passcode: false,
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
        assert_eq!(
            manager.attempt(ADA, false, true, later),
            answer(Verdict::Busy, 0)
        );
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
        manager.attempt(ADA, false, true, now);
        // One at a time.
        assert_eq!(
            manager.attempt(ADA, false, true, now),
            answer(Verdict::Busy, 0)
        );
        assert_eq!(
            pam(&mut manager, Checked::Rejected, now),
            answer(Verdict::Rejected, 0)
        );
        assert_eq!(manager.state(), &State::Greeter { checking: None });
        assert_eq!(
            manager.attempt(ADA, false, true, now),
            [Action::Check {
                account: ADA.into(),
                passcode: false,
            }]
        );
    }

    #[test]
    fn five_rejections_then_the_waits_and_an_acceptance_resets_them() {
        let mut now = Instant::now();
        let (mut manager, _) = Manager::start(now);
        for _ in 0..4 {
            manager.attempt(ADA, false, true, now);
            assert_eq!(
                pam(&mut manager, Checked::Rejected, now),
                answer(Verdict::Rejected, 0)
            );
        }
        manager.attempt(ADA, false, true, now);
        assert_eq!(
            pam(&mut manager, Checked::Rejected, now),
            answer(Verdict::Rejected, 30)
        );
        // Throttled: no worker is started, the wait is said.
        now += Duration::from_secs(10);
        assert_eq!(
            manager.attempt(ADA, false, true, now),
            answer(Verdict::Throttled, 20)
        );
        // Another account is not held back by this one's.
        let other = "oxy-bbbbbbbbbbbbbbbbbbbb";
        assert_eq!(
            manager.attempt(other, false, true, now),
            [Action::Check {
                account: other.into(),
                passcode: false,
            }]
        );
        pam(&mut manager, Checked::Rejected, now);
        // The wait over, a sixth rejection doubles it.
        now += Duration::from_secs(20);
        manager.attempt(ADA, false, true, now);
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
            manager.attempt(ADA, false, true, now);
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
            manager.attempt(ADA, false, true, now);
            assert_eq!(
                pam(&mut manager, Checked::Failed, now),
                answer(Verdict::Rejected, 0)
            );
        }
        // A worker gone without an answer is such a failure.
        manager.attempt(ADA, false, true, now);
        assert_eq!(manager.worker_ended(now), answer(Verdict::Rejected, 0));
        assert_eq!(manager.state(), &State::Greeter { checking: None });
    }

    #[test]
    fn an_account_not_enrolled_is_refused_without_pam_and_throttled_together() {
        let now = Instant::now();
        let (mut manager, _) = Manager::start(now);
        for name in ["root", "nate", "oxy-x", "daemon"] {
            assert_eq!(
                manager.attempt(name, false, false, now),
                answer(Verdict::Rejected, 0)
            );
        }
        assert_eq!(
            manager.attempt("bin", false, false, now),
            answer(Verdict::Rejected, 30)
        );
        assert_eq!(
            manager.attempt("sys", false, false, now),
            answer(Verdict::Throttled, 30)
        );
        // An enrolled account is still checked.
        assert_eq!(
            manager.attempt(ADA, false, true, now),
            [Action::Check {
                account: ADA.into(),
                passcode: false,
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
        manager.attempt(ADA, false, true, now);
        // The first end: started again at once.
        let soon = now + Duration::from_millis(300);
        assert_eq!(
            manager.greeter_ended(soon),
            [
                Action::AbandonCheck,
                Action::StartGreeter {
                    after: Duration::ZERO
                }
            ]
        );
        assert_eq!(manager.state(), &State::Greeter { checking: None });
        // The abandoned check's answer, if it comes, signs nobody in.
        assert!(pam(&mut manager, Checked::Accepted, soon).is_empty());
        assert_eq!(manager.state(), &State::Greeter { checking: None });
        // Having run a minute, at once again.
        let later = soon + Duration::from_secs(60);
        assert_eq!(
            manager.greeter_ended(later),
            [Action::StartGreeter {
                after: Duration::ZERO
            }]
        );
    }

    /// The login screen of 7 November 2026 on the ThinkPad, ending 174 times in three minutes
    /// (PAM failing to set its user's credentials): now 0, 1, 2, 4, 8 … 60 s apart, never given
    /// up on, and a minute's steady run starts over.
    #[test]
    fn a_login_screen_that_keeps_crashing_waits_ever_longer_and_is_never_given_up_on() {
        let start = Instant::now();
        let (mut manager, _) = Manager::start(start);
        let mut now = start;
        let mut starts = Vec::new();
        for _ in 0..10 {
            // It ends a tenth of a second after each start.
            now += Duration::from_millis(100);
            match manager.greeter_ended(now).as_slice() {
                [Action::StartGreeter { after }] => {
                    starts.push(after.as_secs());
                    now += *after;
                }
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(starts, [0, 1, 2, 4, 8, 16, 32, 60, 60, 60]);
        // A run of a minute: its end is a first one again.
        now += Duration::from_secs(60);
        assert_eq!(
            manager.greeter_ended(now),
            [Action::StartGreeter {
                after: Duration::ZERO
            }]
        );
        // Signing out brings it back at once, whatever came before.
        now += Duration::from_millis(100);
        assert_eq!(
            manager.greeter_ended(now),
            [Action::StartGreeter {
                after: Duration::from_secs(1)
            }]
        );
        signed_in(&mut manager, now + Duration::from_secs(1));
        manager.greeter_ended(now + Duration::from_secs(1));
        assert_eq!(
            manager.worker_ended(now + Duration::from_secs(2)),
            [Action::StartGreeter {
                after: Duration::ZERO
            }]
        );
    }

    #[test]
    fn a_session_that_ends_because_of_a_problem_is_said_and_told_to_its_next_session() {
        let now = Instant::now();
        let (mut manager, _) = Manager::start(now);
        signed_in(&mut manager, now);
        manager.greeter_ended(now);
        manager.session_started();
        assert!(!manager.problem_said());
        manager.session_ended(End::Signal(11), 3600, 1_762_790_000);
        assert_eq!(
            manager.worker_ended(now + Duration::from_secs(3600)),
            [Action::StartGreeter {
                after: Duration::ZERO
            }]
        );
        // The login screen says it until someone signs in; its account's next session is told.
        assert!(manager.problem_said());
        assert_eq!(manager.problem_of("someone-else"), None);
        assert_eq!(
            manager.problem_of(ADA).map(Problem::told).as_deref(),
            Some("signal 11 3600 1762790000")
        );
        // A wrong password changes nothing.
        let later = now + Duration::from_secs(7200);
        manager.attempt(ADA, false, true, later);
        pam(&mut manager, Checked::Rejected, later);
        assert!(manager.problem_said());
        assert!(manager.problem_of(ADA).is_some());
        // Once its session starts, it was told: nothing is said or told again.
        signed_in(&mut manager, later + Duration::from_secs(60));
        manager.greeter_ended(later + Duration::from_secs(60));
        manager.session_started();
        assert!(!manager.problem_said());
        assert_eq!(manager.problem_of(ADA), None);
    }

    #[test]
    fn signing_out_or_being_asked_to_end_is_no_problem() {
        let now = Instant::now();
        let (mut manager, _) = Manager::start(now);
        signed_in(&mut manager, now);
        manager.greeter_ended(now);
        manager.session_started();
        manager.session_ended(End::Exited(0), 60, 0);
        manager.session_ended(End::Asked, 60, 0);
        assert!(!manager.problem_said());
        assert_eq!(manager.problem_of(ADA), None);
        // Nothing heard at the login screen counts either.
        let (mut manager, _) = Manager::start(now);
        manager.session_ended(End::Hung, 60, 0);
        assert!(!manager.problem_said());
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
        manager.attempt(ADA, false, true, now);
        let check = asks_online(&mut manager, Checked::Accepted, Local::Accepted, now);
        // Not yet: the login screen is told nothing until the service answers.
        assert!(matches!(
            manager.state(),
            State::Greeter { checking: Some(_) }
        ));
        assert_eq!(
            manager.attempt(ADA, false, true, now),
            answer(Verdict::Busy, 0)
        );
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
        manager.attempt(ADA, false, true, now);
        let check = asks_online(&mut manager, Checked::Rejected, Local::Rejected, now);
        // The worker that rejected it ends, as rejecting workers do: that is expected.
        assert!(manager.worker_ended(now).is_empty());
        // Oxy accepts it, and the service replaced the verifier: a new worker checks it.
        assert_eq!(
            manager.online_checked(check, Online::Accepted, now),
            [Action::Check {
                account: ADA.into(),
                passcode: false,
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
            manager.attempt(ADA, false, true, now);
            let check = asks_online(&mut manager, Checked::Accepted, Local::Accepted, now);
            let actions = manager.online_checked(check, Online::Rejected, now);
            assert_eq!(actions[0], Action::AbandonCheck);
        }
        // Counted: the fifth waits.
        assert_eq!(
            manager.attempt(ADA, false, true, now),
            answer(Verdict::Throttled, 30)
        );
    }

    #[test]
    fn the_service_unreachable_signs_nobody_in_and_is_not_counted() {
        let now = Instant::now();
        let (mut manager, _) = Manager::start(now);
        for _ in 0..10 {
            manager.attempt(ADA, false, true, now);
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
        manager.attempt(ADA, false, true, now);
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
        manager.attempt(ADA, false, true, now);
        let check = asks_online(&mut manager, Checked::Accepted, Local::Accepted, now);
        assert_eq!(manager.worker_ended(now), answer(Verdict::Rejected, 0));
        // The service's late acceptance signs nobody in.
        assert!(manager
            .online_checked(check, Online::Accepted, now)
            .is_empty());
        assert_eq!(manager.state(), &State::Greeter { checking: None });
    }

    #[test]
    fn a_pin_is_checked_under_its_own_service_and_throttle_and_never_online() {
        let now = Instant::now();
        let (mut manager, _) = Manager::start(now);
        assert_eq!(
            manager.attempt(ADA, true, true, now),
            [Action::Check {
                account: ADA.into(),
                passcode: true,
            }]
        );
        // Rejected: no online check, counted on the PIN's throttle.
        assert_eq!(
            manager.checked(Checked::Rejected, now),
            [Action::Answer {
                verdict: Verdict::Rejected,
                wait_s: 0
            }]
        );
        for _ in 0..3 {
            manager.attempt(ADA, true, true, now);
            manager.checked(Checked::Rejected, now);
        }
        manager.attempt(ADA, true, true, now);
        assert_eq!(
            manager.checked(Checked::Rejected, now),
            [Action::Answer {
                verdict: Verdict::Rejected,
                wait_s: 30
            }]
        );
        assert_eq!(
            manager.attempt(ADA, true, true, now),
            [Action::Answer {
                verdict: Verdict::Throttled,
                wait_s: 30
            }]
        );
        // The password's throttle is its own: it is checked, and online.
        assert_eq!(
            manager.attempt(ADA, false, true, now),
            [Action::Check {
                account: ADA.into(),
                passcode: false,
            }]
        );
        manager.checked(Checked::Rejected, now);
        // A right PIN, once its wait is over: accepted at once, no online check, signed in with
        // it (so no keychain is unlocked with it).
        let (mut manager, _) = Manager::start(now);
        manager.attempt(ADA, true, true, now);
        assert_eq!(
            manager.checked(Checked::Accepted, now),
            [Action::Answer {
                verdict: Verdict::Accepted,
                wait_s: 0
            }]
        );
        assert_eq!(manager.handing_over(), Some(ADA));
        assert!(manager.handing_over_with_pin());
        // A PAM that failed was no guess.
        let (mut manager, _) = Manager::start(now);
        for _ in 0..10 {
            manager.attempt(ADA, true, true, now);
            manager.checked(Checked::Failed, now);
        }
        assert!(matches!(
            manager.attempt(ADA, true, true, now)[..],
            [Action::Check { passcode: true, .. }]
        ));
    }
}
