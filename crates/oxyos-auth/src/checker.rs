//! Checking off the compositor's thread: PAM may take seconds (a failure's delay), and the
//! compositor never waits. One password is checked at a time, on a thread of its own; the
//! throttle decides whether one may be tried at all.

use crate::{Authenticator, Outcome, Secret, Throttle};
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// What became of an attempt at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attempt {
    /// The password is being checked: [`Checker::answer`] gives the answer.
    Checking,
    /// Another password is being checked; this one was not.
    Busy,
    /// Attempts must wait this long; this one was not checked.
    Throttled(Duration),
}

/// What a checked password came to, and how long the next attempt must wait (zero: at once).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Answer {
    pub outcome: Outcome,
    pub wait: Duration,
}

pub struct Checker {
    passwords: mpsc::Sender<Secret>,
    answers: mpsc::Receiver<Result<Outcome, crate::Error>>,
    /// Readable once an answer is ready, for the compositor's poll.
    ready: OwnedFd,
    checking: bool,
    throttle: Throttle,
}

impl Checker {
    /// A checker of passwords for `authenticator`'s user, on a thread that lives as long as it.
    pub fn spawn(authenticator: Authenticator) -> std::io::Result<Self> {
        let (ready, signal) = rustix::pipe::pipe_with(
            rustix::pipe::PipeFlags::CLOEXEC | rustix::pipe::PipeFlags::NONBLOCK,
        )?;
        let (passwords, received) = mpsc::channel::<Secret>();
        let (answered, answers) = mpsc::channel();
        std::thread::Builder::new()
            .name("oxyos-auth".into())
            .spawn(move || {
                for secret in received {
                    if answered.send(authenticator.check(secret)).is_err() {
                        break;
                    }
                    let _ = rustix::io::write(&signal, &[1]);
                }
            })?;
        Ok(Self {
            passwords,
            answers,
            ready,
            checking: false,
            throttle: Throttle::default(),
        })
    }

    /// Begins checking `secret`, tried at `now`, unless one is being checked or the throttle
    /// says to wait.
    pub fn attempt(&mut self, secret: Secret, now: Instant) -> Attempt {
        if self.checking {
            return Attempt::Busy;
        }
        if let Some(wait) = self.throttle.wait(now) {
            return Attempt::Throttled(wait);
        }
        if self.passwords.send(secret).is_err() {
            // The thread is gone: nothing is checked, and so nothing unlocks.
            return Attempt::Busy;
        }
        self.checking = true;
        Attempt::Checking
    }

    /// Readable when the password being checked has its answer.
    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.ready.as_fd()
    }

    /// The answer for the password being checked, once there is one, at `now`. A PAM failure
    /// (a broken configuration) is a rejection that the throttle does not count: it was no guess.
    pub fn answer(&mut self, now: Instant) -> Option<Answer> {
        let answer = self.answers.try_recv().ok()?;
        let mut drained = [0; 8];
        while rustix::io::read(&self.ready, &mut drained).is_ok_and(|read| read > 0) {}
        self.checking = false;
        let outcome = match answer {
            Ok(outcome) => {
                self.throttle.record(outcome, now);
                outcome
            }
            Err(error) => {
                eprintln!("[auth] {error}");
                Outcome::Rejected
            }
        };
        Some(Answer {
            outcome,
            wait: self.throttle.wait(now).unwrap_or_default(),
        })
    }
}
