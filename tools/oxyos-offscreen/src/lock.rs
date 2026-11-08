//! The development compositor's answer to the lock screen (docs/adr/0010-lock-screen.mdx): a
//! lock surface's password is checked against the one the scenario gives (`--lock-password`),
//! never with PAM, and throttled as the session's is (`oxyos_auth::Throttle`: the fifth wrong
//! password in a row makes the next wait 30 s), so the lock screen's wait can be seen and heard.
//! With `--lock-pin`, a PIN said as one (`AuthenticatePasscode`) is checked against it, under a
//! throttle of its own, as the session checks a PIN under its own service (ADR 0020 decision 2);
//! never the password as a PIN, nor the PIN as the password.
//! It is a fixture: the session's authentication path is PAM's, and the session never has this
//! one.

use oxyos_compositor::Authentication;
use oxyos_compositor_core::scene::Secret;
use oxyos_protocol::AuthOutcome;
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use std::time::Instant;

pub struct Fixture {
    password: String,
    pin: Option<String>,
    /// What the password submitted came to, and whether it was a PIN, until it is answered.
    checked: Option<(bool, bool)>,
    throttle: oxyos_auth::Throttle,
    pin_throttle: oxyos_auth::Throttle,
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
            pin: None,
            checked: None,
            throttle: oxyos_auth::Throttle::default(),
            pin_throttle: oxyos_auth::Throttle::default(),
            ready,
            signal,
        })
    }
}

impl Fixture {
    /// A PIN said as one unlocks when it is `pin`.
    pub fn with_pin(mut self, pin: Option<String>) -> Self {
        self.pin = pin;
        self
    }
}

impl Authentication for Fixture {
    fn attempt(
        &mut self,
        account: Option<&str>,
        passcode: bool,
        secret: Secret,
    ) -> Option<(AuthOutcome, u32)> {
        if self.checked.is_some() {
            return Some((AuthOutcome::Busy, 0));
        }
        // A lock checks the session's own user, as the session's does: never an account named.
        if account.is_some() {
            return Some((AuthOutcome::Rejected, 0));
        }
        let throttle = if passcode {
            &self.pin_throttle
        } else {
            &self.throttle
        };
        if let Some(wait) = throttle.wait(Instant::now()) {
            return Some((AuthOutcome::Throttled, oxyos_auth::whole_seconds(wait)));
        }
        let right = if passcode {
            self.pin
                .as_ref()
                .is_some_and(|pin| &secret[..] == pin.as_bytes())
        } else {
            &secret[..] == self.password.as_bytes()
        };
        // Answered on the compositor's next turn, as an authentication path off its thread is.
        self.checked = Some((right, passcode));
        let _ = rustix::io::write(&self.signal, &[1]);
        None
    }

    fn as_fd(&self) -> BorrowedFd<'_> {
        self.ready.as_fd()
    }

    fn answer(&mut self) -> Option<(AuthOutcome, u32)> {
        let (right, passcode) = self.checked.take()?;
        let throttle = if passcode {
            &mut self.pin_throttle
        } else {
            &mut self.throttle
        };
        let mut drained = [0; 8];
        while rustix::io::read(&self.ready, &mut drained).is_ok_and(|read| read > 0) {}
        let now = Instant::now();
        let outcome = if right {
            oxyos_auth::Outcome::Accepted
        } else {
            oxyos_auth::Outcome::Rejected
        };
        throttle.record(outcome, now);
        let wait = oxyos_auth::whole_seconds(throttle.wait(now).unwrap_or_default());
        Some(if right {
            (AuthOutcome::Accepted, wait)
        } else {
            (AuthOutcome::Rejected, wait)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tried(fixture: &mut Fixture, password: &str) -> (AuthOutcome, u32) {
        let secret: Secret = password.as_bytes().to_vec().into();
        match fixture.attempt(None, false, secret) {
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

    fn tried_pin(fixture: &mut Fixture, pin: &str) -> (AuthOutcome, u32) {
        let secret: Secret = pin.as_bytes().to_vec().into();
        match fixture.attempt(None, true, secret) {
            Some(answer) => answer,
            None => fixture.answer().expect("answered on the next turn"),
        }
    }

    #[test]
    fn a_pin_unlocks_as_a_pin_alone_under_a_throttle_of_its_own() {
        let mut fixture = Fixture::new("oxyos.correcta".into())
            .unwrap()
            .with_pin(Some("482916".into()));
        // The password said as a PIN, and the PIN as the password, open nothing.
        assert_eq!(
            tried_pin(&mut fixture, "oxyos.correcta"),
            (AuthOutcome::Rejected, 0)
        );
        assert_eq!(tried(&mut fixture, "482916"), (AuthOutcome::Rejected, 0));
        for _ in 0..3 {
            tried_pin(&mut fixture, "000000");
        }
        assert_eq!(
            tried_pin(&mut fixture, "000000"),
            (AuthOutcome::Rejected, 30)
        );
        assert_eq!(tried_pin(&mut fixture, "482916").0, AuthOutcome::Throttled);
        // The password's own throttle is untouched by the PIN's.
        assert_eq!(
            tried(&mut fixture, "oxyos.correcta"),
            (AuthOutcome::Accepted, 0)
        );
        // Without a PIN, none unlocks.
        let mut without = Fixture::new("oxyos.correcta".into()).unwrap();
        assert_eq!(
            tried_pin(&mut without, "482916"),
            (AuthOutcome::Rejected, 0)
        );
    }
}
