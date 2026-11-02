//! The development compositor's answer to the lock screen (docs/adr/0010-lock-screen.mdx): a
//! lock surface's password is checked against the one the scenario gives (`--lock-password`),
//! never with PAM, and throttled as the session's is (`oxyos_auth::Throttle`: the fifth wrong
//! password in a row makes the next wait 30 s), so the lock screen's wait can be seen and heard.
//! It is a fixture: the session's authentication path is PAM's, and the session never has this
//! one.

use oxyos_compositor::Authentication;
use oxyos_compositor_core::scene::Secret;
use oxyos_protocol::AuthOutcome;
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use std::time::{Duration, Instant};

pub struct Fixture {
    password: String,
    /// What the password submitted came to, until it is answered.
    checked: Option<bool>,
    throttle: oxyos_auth::Throttle,
    ready: OwnedFd,
    signal: OwnedFd,
}

impl Fixture {
    pub fn new(password: String) -> std::io::Result<Self> {
        let (ready, signal) = rustix::pipe::pipe_with(
            rustix::pipe::PipeFlags::CLOEXEC | rustix::pipe::PipeFlags::NONBLOCK,
        )?;
        Ok(Self {
            password,
            checked: None,
            throttle: oxyos_auth::Throttle::default(),
            ready,
            signal,
        })
    }
}

impl Authentication for Fixture {
    fn attempt(&mut self, account: Option<&str>, secret: Secret) -> Option<(AuthOutcome, u32)> {
        if self.checked.is_some() {
            return Some((AuthOutcome::Busy, 0));
        }
        // A lock checks the session's own user, as the session's does: never an account named.
        if account.is_some() {
            return Some((AuthOutcome::Rejected, 0));
        }
        if let Some(wait) = self.throttle.wait(Instant::now()) {
            return Some((AuthOutcome::Throttled, seconds(wait)));
        }
        // Answered on the compositor's next turn, as an authentication path off its thread is.
        self.checked = Some(&secret[..] == self.password.as_bytes());
        let _ = rustix::io::write(&self.signal, &[1]);
        None
    }

    fn as_fd(&self) -> BorrowedFd<'_> {
        self.ready.as_fd()
    }

    fn answer(&mut self) -> Option<(AuthOutcome, u32)> {
        let right = self.checked.take()?;
        let mut drained = [0; 8];
        while rustix::io::read(&self.ready, &mut drained).is_ok_and(|read| read > 0) {}
        let now = Instant::now();
        let outcome = if right {
            oxyos_auth::Outcome::Accepted
        } else {
            oxyos_auth::Outcome::Rejected
        };
        self.throttle.record(outcome, now);
        let wait = seconds(self.throttle.wait(now).unwrap_or_default());
        Some(if right {
            (AuthOutcome::Accepted, wait)
        } else {
            (AuthOutcome::Rejected, wait)
        })
    }
}

/// Whole seconds, rounded up, as the session counts a wait: 0.2 s is still a wait.
fn seconds(wait: Duration) -> u32 {
    u32::try_from(wait.as_millis().div_ceil(1000)).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tried(fixture: &mut Fixture, password: &str) -> (AuthOutcome, u32) {
        let secret: Secret = password.as_bytes().to_vec().into();
        match fixture.attempt(None, secret) {
            Some(answer) => answer,
            None => fixture.answer().expect("answered on the next turn"),
        }
    }

    #[test]
    fn the_fifth_wrong_password_in_a_row_makes_the_next_attempt_wait_as_the_session_s_does() {
        let mut fixture = Fixture::new("oxyos.correcta".into()).unwrap();
        for _ in 0..4 {
            assert_eq!(tried(&mut fixture, "mal"), (AuthOutcome::Rejected, 0));
        }
        assert_eq!(tried(&mut fixture, "mal"), (AuthOutcome::Rejected, 30));
        // While it waits nothing is checked, not even the right password.
        let (outcome, wait) = tried(&mut fixture, "oxyos.correcta");
        assert_eq!(outcome, AuthOutcome::Throttled);
        assert!((29..=30).contains(&wait), "{wait}");
    }

    #[test]
    fn the_right_password_unlocks_and_forgets_the_wrong_ones() {
        let mut fixture = Fixture::new("oxyos.correcta".into()).unwrap();
        for _ in 0..4 {
            tried(&mut fixture, "mal");
        }
        assert_eq!(
            tried(&mut fixture, "oxyos.correcta"),
            (AuthOutcome::Accepted, 0)
        );
        assert_eq!(tried(&mut fixture, "mal"), (AuthOutcome::Rejected, 0));
    }
}
